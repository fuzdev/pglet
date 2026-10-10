//! Fuzz coverage for the native parser: arbitrary input must never panic and
//! never overflow the stack — an `Err` is the acceptable outcome for junk input.

use pglet_testing::rng::Xorshift;

use crate::catalog::Schema;
use crate::parser::{parse, parse_expr};

/// Random joins of SQL fragments, biased toward recognized constructs so the Ok
/// path (and nesting) is exercised, not just the early lex-error path.
#[test]
fn fuzz_fragments_never_panic() {
    const PIECES: &[&str] = &[
        "SELECT", "FROM", "WHERE", "AND", "OR", "NOT", "IS", "NULL", "IN", "BETWEEN", "LIKE",
        "ILIKE", "AS", "DISTINCT", "EXISTS", "ARRAY", "CAST", "ANY", "ALL", "AT", "TIME", "ZONE",
        "x", "t", "c.id", "\"Q\"", "1", "1.5", "'s'", "$1", "$2", "(", ")", "[", "]", ",", ".",
        "*", "+", "-", "/", "::", "@>", "<@", "->", "->>", "=", "<>", "<", "<=", ">", ">=", "int8",
        "text", "jsonb", "coalesce", "lower", "now", "count", "true", "false", ";",
    ];
    let mut rng = Xorshift::new(0x9151_2026_2026_1519);
    let schema = Schema::new();
    for _ in 0..30_000 {
        let count = rng.below(16) as usize;
        let mut source = String::new();
        for _ in 0..count {
            source.push_str(PIECES[rng.below(PIECES.len() as u64) as usize]);
            source.push(' ');
        }
        // Any `Result` is acceptable; the contract is no panic.
        let _ = parse_expr(&source, &schema);
        let _ = parse(&source, &schema);
    }
}

/// Arbitrary ASCII (valid UTF-8) — drives the lexer-error and partial-parse paths.
#[test]
fn fuzz_raw_bytes_never_panic() {
    const POOL: &[u8] = b"selectfromwhere x 09()[],.;:'\"$@|+-*/<>=ELU \t\n";
    let mut rng = Xorshift::new(0x2026_5151_5151_2026);
    let schema = Schema::new();
    for _ in 0..30_000 {
        let len = rng.below(40) as usize;
        let source: String = (0..len)
            .map(|_| POOL[rng.below(POOL.len() as u64) as usize] as char)
            .collect();
        let _ = parse_expr(&source, &schema);
        let _ = parse(&source, &schema);
    }
}

/// Deeply nested input must hit the recursion depth-guard and fail loud, not
/// overflow the stack — the guarantee the explicit depth guard upholds without a
/// stack-growth crate.
///
/// It runs on libtest's default 2 MiB thread, where a debug build's nesting reaches the
/// byte check (`crate::depth`) before [`MAX_DEPTH`](super::MAX_DEPTH), so it answers
/// `54001` short of the wall and does **not** measure `MAX_DEPTH` against `pglet_core`'s
/// `MAX_ERROR_SIZE`: with `Error` widened to 96 B, the size recorded as an overflow, it
/// stays green on a debug build (measured). What it holds is that deep nesting fails
/// loud; if it aborts with a stack overflow instead, a bound moved.
#[test]
fn deep_nesting_fails_loud_not_overflow() {
    let schema = Schema::new();
    assert!(parse_expr(&"(".repeat(5000), &schema).is_err());
    assert!(parse_expr(&("NOT ".repeat(5000) + "x"), &schema).is_err());
    assert!(parse_expr(&"-".repeat(5000), &schema).is_err());
}
