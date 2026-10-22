//! The node kinds the parser's operator loop can build around a chain, derived from the
//! builders' own source, and checked against the chain-forming set and the two depth
//! instruments.
//!
//! A tree grows one level a repetition through every node `Parser::operator_loop` wraps
//! around the chain so far, with no parser nesting to bound it — so each such node must
//! be walked from every walker's lean frame (`Expr::chain_operands` names that set), or
//! be charged `crate::depth::DISPATCH_LEVEL_WEIGHT` a level for the big frame it is
//! walked from. A `::regclass` chain is the case an arm list written from the arms'
//! spellings misses: `parse_cast_target` builds a string-function node for it, neither
//! chain-forming nor visible in the spelling, and unguarded it ran out of V8's stack
//! inside the wasm limit.
//!
//! So the set here is **read from the builders**: every closure `operator_loop` hands to
//! `OperatorChain::wrap`, followed through each function it passes the chain operand to,
//! collecting the `Expr` variants constructed on the way. Then:
//!
//! - every kind outside the chain-forming set must be one the weight is known to cover
//!   ([`CHARGED`]) — a new one fails here until it is measured and listed;
//! - every kind must be driven by `tests-web/integration/depth_edge.ts` (the wasm limit,
//!   under both V8 tiers) and by `crates/pglet/tests/stack_depth.rs` (the native bounds),
//!   which is read by recording what [`note`] sees while each instrument's repeated
//!   pieces are parsed.

use std::cell::RefCell;
use std::collections::BTreeSet;

use crate::catalog::Schema;
use crate::planner::{BinaryOp, Expr, Literal};

/// The kinds outside the chain-forming set that the depth weight covers, each measured
/// under both V8 tiers by an `ARMS` entry of `depth_edge.ts` counted at the weight.
const CHARGED: &[&str] = &["StringFn"];

thread_local! {
    /// The kinds [`note`] has seen since [`record`] started, or `None` when not recording.
    static SEEN: RefCell<Option<BTreeSet<String>>> = const { RefCell::new(None) };
}

/// The variant name of `expr`: the leading identifier of its `Debug` form.
fn kind(expr: &Expr) -> String {
    format!("{expr:?}")
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

/// Record the kind of a node the operator loop just built, while [`record`] runs.
pub(super) fn note(node: &Expr) {
    SEEN.with(|seen| {
        if let Some(seen) = seen.borrow_mut().as_mut() {
            seen.insert(kind(node));
        }
    });
}

/// Run `f`, returning every kind the operator loop built during it.
fn record(f: impl FnOnce()) -> BTreeSet<String> {
    SEEN.with(|seen| *seen.borrow_mut() = Some(BTreeSet::new()));
    f();
    SEEN.with(|seen| seen.borrow_mut().take().unwrap_or_default())
}

/// `src` with every comment and every string and character literal blanked to spaces,
/// byte for byte, so braces and parentheses can be matched and names found in code only.
fn code_only(src: &str) -> String {
    let bytes = src.as_bytes();
    let mut out = bytes.to_vec();
    let mut i = 0;
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for b in &mut out[from..to] {
            if *b != b'\n' {
                *b = b' ';
            }
        }
    };
    while i < bytes.len() {
        match bytes[i] {
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                let end = src[i..].find('\n').map_or(bytes.len(), |n| i + n);
                blank(&mut out, i, end);
                i = end;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let end = src[i + 2..]
                    .find("*/")
                    .map_or(bytes.len(), |n| i + 2 + n + 2);
                blank(&mut out, i, end);
                i = end;
            }
            b'"' => {
                let mut j = i + 1;
                while j < bytes.len() && bytes[j] != b'"' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                blank(&mut out, i + 1, j.min(bytes.len()));
                i = j + 1;
            }
            // A character literal (`'{'`, `'\\'`), not a lifetime (`'a`).
            b'\'' if bytes.get(i + 2) == Some(&b'\'') => {
                blank(&mut out, i + 1, i + 2);
                i += 3;
            }
            b'\'' if bytes.get(i + 1) == Some(&b'\\') && bytes.get(i + 3) == Some(&b'\'') => {
                blank(&mut out, i + 1, i + 3);
                i += 4;
            }
            _ => i += 1,
        }
    }
    String::from_utf8(out).unwrap_or_default()
}

/// The index just past the group opened at `open` (a `(`, `{` or `[`).
fn close_of(code: &str, open: usize) -> usize {
    let mut depth = 0usize;
    for (i, b) in code.bytes().enumerate().skip(open) {
        match b {
            b'(' | b'{' | b'[' => depth += 1,
            b')' | b'}' | b']' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
    }
    code.len()
}

/// `name`'s parameter list and body in `code`, if `code` defines a function `name`.
fn function<'a>(code: &'a str, name: &str) -> Option<(Vec<String>, &'a str)> {
    let at = [format!("fn {name}("), format!("fn {name}<")]
        .iter()
        .find_map(|pattern| code.find(pattern.as_str()))?;
    let open_params = at + code[at..].find('(')?;
    let close_params = close_of(code, open_params);
    let params = split_top_level(&code[open_params + 1..close_params - 1])
        .into_iter()
        .filter_map(|param| {
            let (name, _) = param.split_once(':')?;
            let name = name.trim().trim_start_matches("mut ").trim();
            Some(name.to_string())
        })
        .collect();
    let open_body = close_params + code[close_params..].find('{')?;
    Some((params, &code[open_body..close_of(code, open_body)]))
}

/// `list` split at its top-level commas.
fn split_top_level(list: &str) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut depth = 0usize;
    let mut start = 0;
    for (i, b) in list.bytes().enumerate() {
        match b {
            b'(' | b'{' | b'[' | b'<' => depth += 1,
            b')' | b'}' | b']' | b'>' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                parts.push(&list[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    if !list[start..].trim().is_empty() {
        parts.push(&list[start..]);
    }
    parts
}

const fn is_ident(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Whether `word` occurs in `text` as a whole identifier.
fn mentions(text: &str, word: &str) -> bool {
    text.match_indices(word).any(|(at, _)| {
        let before = text.as_bytes().get(at.wrapping_sub(1)).copied();
        let after = text.as_bytes().get(at + word.len()).copied();
        !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
    })
}

/// The `Expr` variants `body` constructs — every `Expr::Name` that is not a pattern: not
/// after a `let`, not inside a `matches!`, and not followed by `=>`, `|` or `if`.
fn constructed(body: &str) -> BTreeSet<String> {
    // `matches!(…)` holds patterns only.
    let mut code = body.to_string();
    while let Some(at) = code.find("matches!(") {
        let end = close_of(&code, at + "matches!".len());
        code.replace_range(at..end, &" ".repeat(end - at));
    }
    let mut out = BTreeSet::new();
    for (at, _) in code.match_indices("Expr::") {
        if at > 0 && is_ident(code.as_bytes()[at - 1]) {
            continue;
        }
        let name: String = code[at + "Expr::".len()..]
            .bytes()
            .take_while(|b| is_ident(*b))
            .map(char::from)
            .collect();
        // An associated function (`Expr::bool_list`) is not a variant.
        if !name.starts_with(|c: char| c.is_ascii_uppercase()) {
            continue;
        }
        if code[..at].trim_end().ends_with("let") {
            continue;
        }
        let mut rest = at + "Expr::".len() + name.len();
        let after = code[rest..].trim_start();
        if after.starts_with(['(', '{']) {
            let open = code.len() - after.len();
            rest = close_of(&code, open);
        }
        let tail = code[rest..].trim_start();
        let pattern = tail.starts_with("=>")
            || (tail.starts_with('|') && !tail.starts_with("||"))
            || tail.starts_with("if ");
        if !pattern {
            out.insert(name);
        }
    }
    out
}

/// Every call in `body` that passes `tracked`, as `(callee, the callee's parameter it
/// lands in)` — a method (`self.name(…)`, `Self::name(…)`) or a free function of this file.
fn calls_passing(code: &str, body: &str, tracked: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let bytes = body.as_bytes();
    for (at, _) in body.match_indices('(') {
        let name_start = body[..at]
            .bytes()
            .rposition(|b| !is_ident(b))
            .map_or(0, |p| p + 1);
        let name = &body[name_start..at];
        if name.is_empty() || !name.starts_with(|c: char| c.is_ascii_lowercase()) {
            continue;
        }
        let prefix = &body[..name_start];
        let method = prefix.ends_with("self.") || prefix.ends_with("Self::");
        let free = name_start == 0 || !matches!(bytes[name_start - 1], b'.' | b':');
        if !(method || free) {
            continue;
        }
        let Some((params, _)) = function(code, name) else {
            continue;
        };
        let args = split_top_level(&body[at + 1..close_of(body, at) - 1]);
        for (i, arg) in args.iter().enumerate() {
            if mentions(arg, tracked)
                && let Some(param) = params.get(i)
            {
                out.push((name.to_string(), param.clone()));
            }
        }
    }
    out
}

/// Collect what `body` builds around `tracked`, following it into every callee.
fn follow(
    code: &str,
    body: &str,
    tracked: &str,
    kinds: &mut BTreeSet<String>,
    visited: &mut BTreeSet<(String, String)>,
) {
    kinds.extend(constructed(body));
    for (callee, param) in calls_passing(code, body, tracked) {
        if visited.insert((callee.clone(), param.clone()))
            && let Some((_, callee_body)) = function(code, &callee)
        {
            follow(code, callee_body, &param, kinds, visited);
        }
    }
}

/// The kinds `operator_loop` can build around the chain, read from `parser::expr`'s
/// source: each `chain.wrap(|left| …)` closure followed through its callees, and the
/// `AND`/`OR` run `chain.push_bool` gathers into `Expr::bool_list`.
fn built_by_the_operator_loop() -> BTreeSet<String> {
    let code = code_only(include_str!("expr.rs"));
    let (_, body) = function(&code, "operator_loop").expect("operator_loop");
    let mut kinds = BTreeSet::new();
    let mut visited = BTreeSet::new();
    for (at, _) in body.match_indices("chain.wrap(") {
        let open = at + "chain.wrap".len();
        let closure = &body[open + 1..close_of(body, open) - 1];
        let (params, arm) = closure
            .trim_start()
            .strip_prefix('|')
            .and_then(|c| c.split_once('|'))
            .expect("a wrap closure");
        follow(&code, arm, params.trim(), &mut kinds, &mut visited);
    }
    if body.contains("chain.push_bool(") {
        let leaf = || Expr::Literal(Literal::Int(1));
        let run = Expr::bool_list(BinaryOp::And, (0..100).map(|_| leaf()).collect());
        kinds.insert(kind(&run));
    }
    kinds
}

/// The chain-forming kinds, read from `Expr::chain_operands`'s own arms.
fn chain_forming() -> BTreeSet<String> {
    let code = code_only(include_str!("../planner/expr.rs"));
    let (_, body) = function(&code, "chain_operands").expect("chain_operands");
    body.match_indices("Self::")
        .map(|(at, _)| {
            body[at + "Self::".len()..]
                .bytes()
                .take_while(|b| is_ident(*b))
                .map(char::from)
                .collect()
        })
        .collect()
}

/// Every piece `file` repeats — each `rep("…", …)` literal — parsed as a chain behind a
/// few seeds, and the kinds the operator loop built while parsing them.
fn driven_by(file: &str) -> BTreeSet<String> {
    let path = format!("{}/../../{file}", env!("CARGO_MANIFEST_DIR"));
    let source = std::fs::read_to_string(&path).expect(file);
    let mut pieces = Vec::new();
    for (at, _) in source.match_indices("rep(\"") {
        let rest = &source[at + "rep(\"".len()..];
        let mut piece = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            match c {
                '"' => break,
                '\\' => piece.extend(chars.next()),
                c => piece.push(c),
            }
        }
        pieces.push(piece);
    }
    assert!(pieces.len() > 10, "{file}: {} pieces", pieces.len());
    let mut schema = Schema::new();
    if let Ok(Some(plan)) = super::parse_single("CREATE TABLE t (a INT)", &schema) {
        let _ = crate::executor::execute(&plan, &mut schema);
    }
    record(|| {
        for piece in &pieces {
            for seed in ["1", "a", "true", "'a'", "'{}'::jsonb", "now()"] {
                let sql = format!("SELECT {seed}{piece}{piece} FROM t");
                let _ = super::parse_single(&sql, &schema);
            }
        }
    })
}

#[test]
fn the_derivation_reads_what_the_arms_build() {
    // The scan is non-vacuous: it finds the arms' kinds, a string-function node among
    // them, and nothing an operand parser builds.
    let built = built_by_the_operator_loop();
    for expected in [
        "BinaryOp",
        "Cast",
        "CastTypmod",
        "InList",
        "AnyAll",
        "StringFn",
    ] {
        assert!(
            built.contains(expected),
            "{expected} missing from {built:?}"
        );
    }
    for operand in ["Literal", "Column", "FunctionCall", "Case", "Subquery"] {
        assert!(!built.contains(operand), "{operand} in {built:?}");
    }
    let chain = chain_forming();
    for expected in ["BinaryOp", "Cast", "InList", "Not"] {
        assert!(
            chain.contains(expected),
            "{expected} missing from {chain:?}"
        );
    }
    // And what `note` records is what the derivation names.
    let seen = record(|| {
        let _ = super::parse_single("SELECT 't'::regclass::text IS NULL", &Schema::new());
    });
    assert!(seen.contains("StringFn") && seen.contains("Cast") && seen.contains("IsNull"));
}

#[test]
fn every_kind_the_operator_loop_builds_is_chain_forming_or_charged() {
    let built = built_by_the_operator_loop();
    let chain = chain_forming();
    let outside: BTreeSet<&str> = built
        .iter()
        .map(String::as_str)
        .filter(|kind| !chain.contains(*kind))
        .collect();
    assert_eq!(
        outside,
        CHARGED.iter().copied().collect(),
        "a kind the operator loop builds is outside the chain-forming set \
         (`Expr::chain_operands`): take it in every walker's lean frame, or measure it at \
         `DISPATCH_LEVEL_WEIGHT` under both V8 tiers (an `ARMS` entry of `depth_edge.ts`) \
         and list it in `CHARGED`"
    );
}

#[test]
fn every_kind_the_operator_loop_builds_is_driven_by_both_depth_instruments() {
    let built = built_by_the_operator_loop();
    for file in [
        "tests-web/integration/depth_edge.ts",
        "crates/pglet/tests/stack_depth.rs",
    ] {
        let driven = driven_by(file);
        let missing: Vec<&String> = built.iter().filter(|k| !driven.contains(*k)).collect();
        assert!(
            missing.is_empty(),
            "{file} drives no chain through {missing:?}"
        );
    }
}
