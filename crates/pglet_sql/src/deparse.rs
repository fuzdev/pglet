//! The **`Expr` deparser**: a persisted planner [`Expr`] rendered back to SQL.
//!
//! One implementation, six consumers in **two modes**, and that is the whole reason it
//! lives in this crate rather than beside any of them. `pglet::dump` emits it into a
//! portable SQL script and `executor::ddl` runs it as a **probe**, discarding the
//! string — the DDL-time over-rejection that keeps every other consumer from reaching a
//! declining arm, at each of the four sites an expression is persisted from
//! (`cook_column_default` for a column `DEFAULT`, `validate_stored_expr_renders` for a
//! CHECK, an index predicate and an index key part). Those two take
//! [`RenderMode::Portable`]. The four introspection surfaces —
//! `information_schema.columns.column_default` and `.check_constraints.check_clause`,
//! `pg_get_constraintdef`, `pg_indexes.indexdef` — take [`RenderMode::Catalog`], which
//! prints what 18.6's `pg_get_expr` prints for the tree **18.6's parse analysis would
//! have stored**, not a faithful rendering of pglet's own. The full statement of why
//! those are two different strings for the same source, and of every rule the catalog
//! mode reconstructs, is `docs/deparse.md`.
//!
//! One exhaustive match over both modes, deliberately: a second renderer beside this
//! one would be the duplicated-rules seam this engine keeps paying for, and the failure
//! would be quiet in the direction that matters, since nothing compares a dump's
//! spelling with an introspecting client's.
//!
//! The `parse(render(e)) == e` round-trip invariant, its corpus, and the
//! exhaustiveness argument are stated on [`render_expr`] — and they are the *portable*
//! mode's contract alone; the catalog mode is free of them because nothing re-parses a
//! catalog view. The corpus test itself
//! lives in `pglet::dump`'s tests, because recording an `Expr` needs a `Database`
//! — which is why the per-family name tables below are `pub` rather than
//! `#[cfg(test)]`.

use crate::catalog::pg_type_modifier_spelling;
use crate::lexer::{is_safe_bare_ident, quote_ident};
use std::fmt::Write as _;

use crate::planner::{
    BinaryOp, BuiltinFn, Expr, LeafScope, Literal, MathFunc, StringFunc, Typmod, VariadicFunc,
};
use crate::types::TypeOid;
use crate::types::type_oid_to_sql_name;
use crate::{Error, Result};

/// The `42704` error for a type with no round-trippable column spelling — fail loud
/// rather than emit a wrong or missing type.
///
/// **Called on both of [`type_oid_sql`]'s paths; neither can currently produce it.**
/// The unreachable thing is the *failure*, not the invocation: `type_oid_to_sql_name`
/// matches every `TypeOid` exhaustively and answers `Some` for every scalar
/// (`interval` included — it is a real column type), reserving `None` for the array
/// OIDs alone; and [`type_oid_sql`] never hands it an array, because it resolves one
/// through `array_element` — total over the array OIDs — to the scalar element first.
/// So both `ok_or_else` arms are dead **as long as those two facts hold together**,
/// and this is the guard for the day one stops: a future `TypeOid` landing with no
/// DDL spelling, or an array whose element OID is not itself spellable.
pub fn unsupported_type(oid: TypeOid) -> Error {
    Error::UnsupportedType {
        type_name: format!("{oid:?}"),
    }
}

/// The round-trippable DDL spelling for a `TypeOid` (scalar name, or `<element>[]`
/// for a one-dimensional array), failing loud on a type with no column form.
pub fn type_oid_sql(oid: TypeOid) -> Result<String> {
    if let Some(elem) = oid.array_element() {
        let elem_name = type_oid_to_sql_name(elem).ok_or_else(|| unsupported_type(elem))?;
        return Ok(format!("{elem_name}[]"));
    }
    type_oid_to_sql_name(oid)
        .map(ToString::to_string)
        .ok_or_else(|| unsupported_type(oid))
}

/// Which of the two spellings a rendering wants — one match, two contexts.
///
/// The two are not a style choice, and neither substitutes for the other. pglet stores
/// the expression **as written**, where 18.6's `pg_attrdef` / `pg_constraint` hold the
/// post-parse-analysis tree: every literal typed, every no-op cast already gone, an
/// implicit coercion node wherever one was needed. So a faithful rendering of pglet's
/// tree and 18.6's `pg_get_expr` of its own tree are different strings for the same
/// source, and a single renderer cannot be both.
///
/// - [`Self::Portable`] renders pglet's tree **faithfully** — the dump and the DDL-time
///   render probe. Its contract is `parse(render(e)) == e` (see [`render_expr`]); a
///   rendering that added or dropped a node would restore a database whose stored
///   expressions are a different shape, which the planner's partial-index implication
///   check compares structurally.
/// - [`Self::Catalog`] renders what 18.6's `pg_get_expr` would print **for the tree
///   18.6 would have stored** — `information_schema.columns.column_default`,
///   `check_clause`, `pg_get_constraintdef`, `pg_indexes.indexdef`. Nothing re-parses
///   it, so it is free to spell the coercions pglet never stored; that is the whole
///   licence this mode has and the reason the dump does not share it.
///
/// The scope resolves an `Expr::Column`'s type, which is what an *unknown* literal in a
/// sibling position resolves to (`transformExpr`'s rule, reproduced by
/// [`operand_type`]). A `DEFAULT` names no column and passes `PlannerScope::none`; a
/// CHECK or index predicate passes its own relation.
// `LeafScope` is a trait object with no `Debug` bound, so the enum carries a manual
// one-word `Debug` naming the variant rather than a derive that cannot see through it.
#[derive(Clone, Copy)]
pub enum RenderMode<'a> {
    /// Round-trip-faithful: pglet's own tree, re-parsing to the identical `Expr`.
    Portable,
    /// 18.6's `pg_get_expr` spelling, over the named relation's column types.
    Catalog(&'a dyn LeafScope),
}

impl std::fmt::Debug for RenderMode<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Portable => "Portable",
            Self::Catalog(_) => "Catalog",
        })
    }
}

/// Render a persisted planner `Expr` back to SQL.
///
/// The covered subset is what a `CHECK` constraint, an index key or predicate, and a
/// column `DEFAULT` can hold; anything outside it fails loud rather than emit SQL
/// that would diverge on reload.
///
/// The match is **exhaustive on purpose** — no catch-all arm. Every unrenderable
/// variant declines by name through [`unrenderable`], so adding an `Expr` variant
/// is a compile error here rather than a silent inheritance of "decline". That
/// distinction is load-bearing: an `Expr` variant reaches this function only by
/// having been *persisted* into the catalog, and a variant that a catch-all
/// declined would brick `dump_sql` for the **whole database**, not just the
/// offending table (all four of the dump's call sites `?`-propagate out of one
/// `Result` — the column default, the CHECK, the index key and the index predicate).
/// The `Expr` tree walkers (`for_each_child` / `map_children`) are exhaustive for the
/// same reason.
///
/// # The invariant: `parse(render(e)) == e`
///
/// Each arm emits a spelling that re-parses to the **identical** variant, not
/// merely a semantically equivalent one. A stored expression is compared
/// *structurally* by the planner's partial-index implication check, so rendering
/// [`StringFunc::Position`] as `strpos(…)` — which evaluates identically — would
/// restore as a different `Expr` and silently change which queries the restored
/// index serves. `Expr::UnaryPlus` makes the same argument below for its wrapper.
/// `render_expr_round_trips_to_the_identical_variant` is the property test, and
/// `round_trip_corpus_covers_every_renderable_form` is what keeps its corpus from
/// silently falling behind the arms below.
///
/// Where a special-form grammar cannot take an arbitrary operand, an arm falls back
/// to a semantically identical spelling and says so in its own comment. There is
/// exactly one such fallback today — the non-literal `EXTRACT` field, see
/// [`render_extract`].
///
/// # More permissive than Postgres — rendered anyway
///
/// Postgres applies **no** volatility gate to a `CHECK` (`doc/src/sgml/ddl.sgml`
/// documents this as deliberate), so `CHECK (a <> gen_random_uuid())` loads into
/// stock Postgres fine. But `CREATE INDEX` *does* require IMMUTABLE for both
/// expression keys and predicates (`42P17`, `indexcmds.c`) and pglet does not — so
/// a pglet database can hold an index expression stock Postgres rejects on reload.
/// (Two members are refused as 18.6 refuses them: a conversion to an enum, whose node
/// names the type, and `random()` — `executor::ddl`'s `refuse_mutable_index_functions`.)
/// The session-information forms join `now()` / `gen_random_uuid()` / `CURRENT_DATE`
/// on that surface — `CREATE INDEX ix ON t ((current_user))` and
/// `… (a) WHERE a = current_user` are accepted here and are `42P17 functions in index
/// {expression,predicate} must be marked IMMUTABLE` there, measured — and they widen
/// what the permissiveness costs: such a key's value now depends on **which
/// connection inserted the row**, where the clock family only depended on when.
/// It is rendered faithfully regardless: the divergence is pglet's DDL
/// permissiveness, and declining here would produce the whole-database dump outage
/// this function's exhaustiveness exists to prevent.
///
/// Parenthesization follows Postgres's own deparser (`ruleutils.c`): maximal, never
/// omitting a paren it isn't certain of.
pub fn render_expr(expr: &Expr) -> Result<String> {
    render_expr_mode(expr, RenderMode::Portable)
}

/// Render a persisted planner `Expr` as 18.6's `pg_get_expr` would print it, resolving
/// column types (and therefore *unknown* literals) against `scope`.
///
/// The four catalog surfaces call this; the dump and the DDL render probe call
/// [`render_expr`]. See [`RenderMode`] for why they are not the same string.
///
/// # Errors
///
/// The same declines [`render_expr`] raises — the arms are one match.
pub fn render_catalog_expr(expr: &Expr, scope: &dyn LeafScope) -> Result<String> {
    render_expr_mode(expr, RenderMode::Catalog(scope))
}

/// The one rendering match, over both [`RenderMode`]s.
///
/// [`render_expr`] and [`render_catalog_expr`] are its two entry points; a catalog
/// surface that already holds a `RenderMode` — the constraint renderer shared with the
/// dump — calls it directly.
///
/// # Errors
///
/// [`Error::NotImplemented`] for an `Expr` variant with no portable spelling.
pub fn render_expr_mode(expr: &Expr, mode: RenderMode<'_>) -> Result<String> {
    crate::depth::check_stack()?;
    // The chain-forming variants recurse from this frame and format in one of their
    // own, so a chain costs this frame a level, not the whole match's (`crate::depth`).
    match expr {
        Expr::BinaryOp { left, op, right } => render_binary_op(left, *op, right, mode),
        Expr::Negate(inner)
        | Expr::UnaryPlus(inner)
        | Expr::Not(inner)
        | Expr::IsNull(inner)
        | Expr::IsNotNull(inner)
        | Expr::BooleanTest { expr: inner, .. } => {
            let rendered = render_expr_mode(inner, mode)?;
            Ok(wrap_prefix_postfix(expr, &rendered))
        }
        // A cast to an enum parses to a `Cast` to `text` over the conversion node
        // (`planner::build_enum_cast`), and the pair is **one** written cast: the node
        // renders as `x::<enum>` and the `text` cast around it renders nothing, or the
        // text would re-parse to a second cast the stored tree does not hold.
        Expr::Cast {
            expr: inner,
            target_type: TypeOid::Text,
        } if crate::planner::enum_in_parts(inner).is_some() => render_expr_mode(inner, mode),
        Expr::Cast {
            expr: inner,
            target_type,
        } => render_cast(inner, *target_type, None, mode),
        Expr::CastTypmod {
            expr: inner,
            type_oid,
            typmod,
        } => {
            let spelling = typmod_spelling(*typmod);
            render_cast(inner, *type_oid, Some(&spelling), mode)
        }
        // A `sub_select` node's right operand is a scalar subquery, which declines.
        Expr::AnyAll {
            left,
            op,
            right,
            all,
            sub_select: _,
        } => {
            let rendered = render_expr_mode(left, mode)?;
            render_any_all(&rendered, *op, right, *all, mode)
        }
        // The left operand is rendered once, here, and spliced into every comparison
        // the catalog rendering spells: rendering it again per list member cost a nested
        // `IN` chain a render per member per level — exponential in its depth.
        Expr::InList {
            expr: inner,
            list,
            negated,
        } => {
            let rendered = if takes_position_type(inner, mode) {
                None
            } else {
                Some(render_expr_mode(inner, mode)?)
            };
            render_in_list(inner, rendered.as_deref(), list, *negated, mode)
        }
        _ => render_expr_mode_rest(expr, mode),
    }
}

/// A typmod's parenthesized spelling after its type name.
#[inline(never)]
fn typmod_spelling(typmod: Typmod) -> String {
    match typmod {
        Typmod::Numeric { precision, scale } => format!("({precision},{scale})"),
        Typmod::Length(n) => format!("({n})"),
        Typmod::Precision(p) => format!("({p})"),
    }
}

/// Whether `expr` renders differently by the type its position resolves an *unknown* to
/// ([`render_operand`]'s shapes) — none of them a chain.
const fn takes_position_type(expr: &Expr, mode: RenderMode<'_>) -> bool {
    matches!(mode, RenderMode::Catalog(_))
        && matches!(expr, Expr::Literal(_) | Expr::Array(_) | Expr::Coalesce(_))
}

/// The spelling around an already rendered operand of a one-operand node
/// [`render_expr_mode`] walks itself.
#[inline(never)]
fn wrap_prefix_postfix(expr: &Expr, operand: &str) -> String {
    match expr {
        Expr::Negate(_) => format!("(- {operand})"),
        // Rendered **faithfully** rather than folded to its operand. The wrapper is
        // semantically transparent, so `1` would evaluate identically — but the
        // persisted shape is what the planner's partial-index implication check
        // compares structurally, so dropping the `+` on restore would silently
        // change which queries a `WHERE +a > 0` index serves. Mirrors `Negate`.
        Expr::UnaryPlus(_) => format!("(+ {operand})"),
        Expr::Not(_) => format!("(NOT {operand})"),
        Expr::IsNull(_) => format!("({operand} IS NULL)"),
        Expr::IsNotNull(_) => format!("({operand} IS NOT NULL)"),
        // Faithful, as 18.6's `pg_get_constraintdef` renders it: a stored
        // `CHECK (b IS TRUE)` reads back `CHECK ((b IS TRUE))` there, so the dump
        // round-trips through stock Postgres and back into pglet as the same node.
        Expr::BooleanTest { test, .. } => format!("({operand} {})", test.sql()),
        _ => operand.to_owned(),
    }
}

#[inline(never)]
fn render_expr_mode_rest(expr: &Expr, mode: RenderMode<'_>) -> Result<String> {
    Ok(match expr {
        // A **top-level** literal has no sibling to resolve an *unknown* against, so it
        // renders as one. Every position that does carry a resolution — a cast operand, a
        // comparison's other side, an `IN` element — reaches [`render_const`] through its
        // own arm with the type it resolved to.
        Expr::Literal(lit) => render_const(lit, None, mode),
        Expr::Column { table, name } => table.as_ref().map_or_else(
            || quote_ident(name),
            |t| format!("{}.{}", quote_ident(t), quote_ident(name)),
        ),
        Expr::BinaryOp { left, op, right } => render_binary_op(left, *op, right, mode)?,
        // Walked by `render_expr_mode` itself: never here.
        Expr::Negate(_)
        | Expr::UnaryPlus(_)
        | Expr::Not(_)
        | Expr::IsNull(_)
        | Expr::IsNotNull(_)
        | Expr::BooleanTest { .. }
        | Expr::Cast { .. }
        | Expr::CastTypmod { .. }
        | Expr::AnyAll { .. }
        | Expr::InList { .. } => render_expr_mode(expr, mode)?,
        Expr::Coalesce(args) => render_coalesce(args, None, mode)?,
        Expr::Lower(e) => {
            // `lower(text)` — its one argument position is `text`, so an *unknown*
            // literal there resolves to it (`lower('AB')` reads `lower('AB'::text)`).
            let ty = matches!(mode, RenderMode::Catalog(_)).then_some(TypeOid::Text);
            format!("lower({})", render_operand(e, ty, mode)?)
        }
        // `pg_column_size("any")` — its argument position takes any type, so the
        // argument renders as written
        Expr::ColumnSize { arg, .. } => {
            format!("pg_column_size({})", render_operand(arg, None, mode)?)
        }
        Expr::Array(items) => render_array(items, None, mode)?,
        Expr::Case {
            operand,
            when_clauses,
            else_result,
        } => render_case(
            operand.as_deref(),
            when_clauses,
            else_result.as_deref(),
            mode,
        )?,

        // --- The scalar-function families. Each is persisted-reachable at all three
        // stored-expression sites (`CHECK (abs(a) < 10)` is ordinary DDL pglet
        // accepts and stores), so declining any of them bricked `dump_sql` for the
        // whole database. Each renders to the spelling that re-parses to the same
        // variant — see the per-helper comments for the ones where that is not the
        // obvious `name(args…)`.
        Expr::MathFn { func, args } => render_call(math_fn_name(*func), args, mode)?,
        Expr::StringFn { func, args } => render_string_fn(*func, args, mode)?,
        Expr::VariadicFn { func, args } => render_variadic_fn(*func, args, mode)?,
        Expr::JsonbBuildObject(args) => render_call("jsonb_build_object", args, mode)?,
        Expr::Builtin(func) => builtin_sql(*func).to_string(),
        Expr::Extract {
            source,
            field,
            retnumeric,
        } => render_extract(source, field, *retnumeric, mode)?,

        // --- Declined by name. Listing each keeps the match exhaustive, so a new
        // `Expr` variant must be classified here instead of inheriting a catch-all.
        //
        // Three groups, none of which a *persisted* expression can be — and the
        // third is the one whose unreachability this function does not own:
        //  - rejected by DDL in every stored-expression context, so one can never be
        //    recorded: subqueries (`0A000` — the scalar, `IN (SELECT …)` and `EXISTS`
        //    spellings alike, which also seals the only route a plan-bearing `Expr`
        //    had into the catalog), window functions (`42P20`), aggregates (`42803`),
        //    set-returning `unnest` (`0A000`), plus the three the binding check in
        //    `executor::ddl` closes — `DEFAULT` (`42601`), a `$N` parameter
        //    (`42P02`), and a column reference the table does not have (an
        //    `Expr::Column`, so it declines there rather than here). Each measured at
        //    all four sites, identical to 18.6 at each. The column reference is the
        //    one whose *code* moves between them: `42703` at a CHECK and both index
        //    positions, and `0A000 cannot use column reference in DEFAULT expression`
        //    at a column `DEFAULT`, where no column may be named at all — both
        //    engines, both codes. A **sequence function** used to be listed here and
        //    is not one of these: it renders (see the arm below), its `nextval`
        //    spelling is *accepted* in a `DEFAULT` on both engines, and pglet's
        //    `0A000` at the other three sites is a position gate rather than a
        //    persistence gate (`EDGES.md` row 50; the `currval`/`setval` spellings are
        //    that same gate at the `DEFAULT` site, row 47);
        //  - `Alias`, plan-internal: no stored-expression grammar can spell an `AS`
        //    inside an expression, so `CHECK ((a AS z) > 0)` and
        //    `CREATE INDEX … ((a AS z))` are `42601` in the parser;
        //  - **`Bound`, which SQL text *can* produce** — `parser::expr`'s
        //    `parse_interval_literal` carries an `INTERVAL '…'` literal as one, and
        //    is the only live producer. (`parser::ddl`'s `column_default_to_expr`
        //    constructs one too, from a `ColumnDefault::Literal` carrying a `Value`
        //    outside the five its own classifier produces — dead by that range, and
        //    confined to the `DEFAULT` site the probe already covers. The qualifier
        //    is here so the "only producer" sentence survives a grep.) Nothing about
        //    the variant makes it unreachable: it is kept out of the catalog by the
        //    **render probe**, at each of the four sites and nowhere else —
        //    `executor::ddl`'s
        //    `cook_column_default` for a column `DEFAULT`, and its
        //    `validate_stored_expr_renders` for a CHECK, an index predicate and an
        //    index key part. Before that probe covered the last three,
        //    `CREATE TABLE ck1 (t TIMESTAMPTZ, CHECK (t > now() - interval '1 day'))`
        //    — ordinary DDL 18.6 accepts — persisted one and took the **whole
        //    database's** dump down with it. A reader-side claim of unreachability
        //    has to name the writer-side gate it rests on; this is that naming.
        //
        // So every arm below is a can't-happen that fails loud instead of guessing.
        Expr::Default => unrenderable("DEFAULT")?,
        Expr::Alias { .. } => unrenderable("an aliased expression")?,
        Expr::Aggregate { .. } => unrenderable("an aggregate call")?,
        Expr::Subquery { .. } => unrenderable("a scalar subquery")?,
        Expr::InSubquery { .. } => unrenderable("an IN (SELECT …) subquery")?,
        Expr::Exists { .. } => unrenderable("an EXISTS subquery")?,
        Expr::Window { .. } => unrenderable("a window function call")?,
        Expr::Parameter { .. } => unrenderable("a parameter placeholder")?,
        Expr::Bound(_) => unrenderable("a plan-internal bound value")?,
        Expr::Unnest(_) => unrenderable("unnest(…)")?,
        // A sequence call **is** renderable, and it has to be: a column
        // `DEFAULT nextval('s') + 1` persists one inside a `ColumnDefault::Expr`, and
        // declining here would take the whole database's dump down with it. The bare
        // spelling is emitted rather than pg_dump's `nextval('s'::regclass)` because
        // this rendering has to re-parse through the **general expression grammar**,
        // which refuses that spelling: `parser::parse_cast_target` branches on the
        // canonical type name `parse_type_name` returned — `regclass` exactly — and
        // lowers it to a `StringFn`, which `planner::sequence_name_from_arg`'s cast
        // look-through does not match, so `SELECT nextval('s'::regclass)` is
        // `0A000 not implemented: nextval() requires a string-literal sequence name`
        // where 18.6 answers `1` — the recorded over-rejection.
        //
        // `quote_name_literal` is still the right quoting: the literal is read back by
        // the same `reduce_name_literal` grammar the identifier positions use, so a
        // name needing quotes must carry them inside the string or it resolves to a
        // different relation.
        //
        // Only the *closed* `ColumnDefault::NextVal` arm keeps the `::regclass`
        // suffix (see `pglet::dump`'s `render_default`), because that one is written
        // into a column-constraint `DEFAULT` position, where the token-level
        // classifier reads it.
        Expr::SequenceFn { func, seq, args } => {
            // The catalog rendering carries `::regclass`, which the portable one cannot:
            // 18.6's `pg_get_expr` prints `nextval('s'::regclass)` and nothing re-parses
            // a catalog view, while a dump has to go back through the general expression
            // grammar, which answers that spelling `0A000` from the sequence-name
            // look-through — the recorded over-rejection stated above.
            let name = match mode {
                RenderMode::Portable => quote_name_literal(seq),
                RenderMode::Catalog(_) => format!("{}::regclass", quote_name_literal(seq)),
            };
            let mut rendered = vec![name];
            for arg in args {
                rendered.push(render_expr_mode(arg, mode)?);
            }
            format!("{}({})", func.name(), rendered.join(", "))
        }
    })
}

/// `name(arg, arg, …)` — the ordinary function-call spelling, arity-agnostic (the
/// arity was already checked when the call was built, and re-parsing re-checks it).
fn render_call(name: &str, args: &[Expr], mode: RenderMode<'_>) -> Result<String> {
    let rendered: Result<Vec<String>> = args.iter().map(|a| render_expr_mode(a, mode)).collect();
    Ok(format!("{name}({})", rendered?.join(", ")))
}

/// The SQL spelling of a [`MathFunc`] — the name `planner::build_call` maps back to
/// the same variant.
///
/// `ceil` is emitted for [`MathFunc::Ceil`] even though `ceiling` also builds it;
/// both re-parse identically, and `ceil` is what Postgres deparses.
pub const fn math_fn_name(func: MathFunc) -> &'static str {
    match func {
        MathFunc::Abs => "abs",
        MathFunc::Ceil => "ceil",
        MathFunc::Floor => "floor",
        MathFunc::Round => "round",
        MathFunc::Trunc => "trunc",
        MathFunc::Sqrt => "sqrt",
        MathFunc::Sign => "sign",
    }
}

/// Every [`MathFunc`], for the corpus completeness check. See `pglet::dump`'s `ROUND_TRIP_CORPUS`.
pub const MATH_FUNCS: [MathFunc; 7] = [
    MathFunc::Abs,
    MathFunc::Ceil,
    MathFunc::Floor,
    MathFunc::Round,
    MathFunc::Trunc,
    MathFunc::Sqrt,
    MathFunc::Sign,
];

/// Render a [`Expr::VariadicFn`] call the way 18.6 spells the node its parse analysis
/// built — which for three of the five is not a function call at all.
///
/// **Two of the five names are `pg_proc` functions and three are keyword constructs.**
/// `concat` / `concat_ws` are `pg_proc` entries and print through
/// `generate_function_name`, lowercase. `greatest` / `least` build a `MinMaxExpr` and
/// `nullif` a `NullIfExpr`, and `get_rule_expr` prints each as the **uppercase keyword**
/// — `GREATEST(a, b)`, `LEAST(a, b)`, `NULLIF(a, b)` — exactly as it prints
/// `COALESCE(`, which is why [`render_coalesce`] was already uppercase and these three
/// were the family's odd members out (measured on 18.6, `COALESCE` beside them as the
/// control).
///
/// **The unknown-resolving rule splits the three the same way 18.6's parse analysis
/// does**, and the split is not cosmetic:
///
/// - `concat_ws(sep, VARIADIC "any")` declares `text` for its **first** argument in
///   every `pg_proc` overload and nothing for the rest, so an unknown there resolves —
///   `concat_ws('-', s, 'x')` reads `concat_ws('-'::text, s, 'x')` (measured).
///   `concat` is `VARIADIC "any"` throughout and resolves nothing.
/// - `GREATEST` / `LEAST` are `select_common_type` positions over **all** their
///   arguments (`transformMinMaxExpr`), so they take [`common_element_type`] — the same
///   rule `COALESCE` and `ARRAY[…]` take, `Varchar` **not** folded to `Text`:
///   `LEAST('a', s)` over a `text` column reads `LEAST('a'::text, s)` and
///   `LEAST(v, 'x')` over a `varchar(10)` one reads
///   `(LEAST(v, 'x'::character varying))::text`, the outer relabel being row 90's.
/// - `NULLIF` is not a common-type position at all: `transformAExprNullIf` resolves the
///   `=` **operator** over the pair, so its unknown takes the *sibling's* type through
///   [`unknown_resolves_to`], `Varchar` folded — `NULLIF(v, 'x')` reads
///   `NULLIF((v)::text, 'x'::text)` where `LEAST`'s literal stays `character varying`.
///   All four measured on 18.6; a two-argument list is what the parser accepts for
///   `nullif`, and any other arity falls back to the plain rendering rather than
///   indexing.
///
/// A pair whose known members disagree still declines (`GREATEST(n, 1)` over a
/// `numeric` column reads `GREATEST(n, (1)::numeric)` there and `GREATEST(n, 1)` here)
/// — `EDGES.md` row 90's class, which is where the *whole* family was first filed and
/// where only that residual belongs.
fn render_variadic_fn(func: VariadicFunc, args: &[Expr], mode: RenderMode<'_>) -> Result<String> {
    let RenderMode::Catalog(scope) = mode else {
        return render_call(variadic_fn_name(func), args, mode);
    };
    match func {
        VariadicFunc::Concat => render_call(variadic_fn_name(func), args, mode),
        VariadicFunc::ConcatWs => {
            let rendered: Result<Vec<String>> = args
                .iter()
                .enumerate()
                .map(|(i, a)| render_operand(a, (i == 0).then_some(TypeOid::Text), mode))
                .collect();
            Ok(format!("concat_ws({})", rendered?.join(", ")))
        }
        VariadicFunc::Greatest | VariadicFunc::Least => {
            let common = common_element_type(args, scope);
            let rendered: Result<Vec<String>> = args
                .iter()
                .map(|a| render_operand(a, common, mode))
                .collect();
            let keyword = if matches!(func, VariadicFunc::Greatest) {
                "GREATEST"
            } else {
                "LEAST"
            };
            Ok(format!("{keyword}({})", rendered?.join(", ")))
        }
        VariadicFunc::NullIf => {
            let rendered: Result<Vec<String>> = match args {
                [left, right] => {
                    let lt = operand_type(left, scope);
                    let rt = operand_type(right, scope);
                    Ok(vec![
                        render_operand(left, unknown_resolves_to(BinaryOp::Eq, rt), mode)?,
                        render_operand(right, unknown_resolves_to(BinaryOp::Eq, lt), mode)?,
                    ])
                }
                other => other.iter().map(|a| render_expr_mode(a, mode)).collect(),
            };
            Ok(format!("NULLIF({})", rendered?.join(", ")))
        }
    }
}

/// The SQL spelling of a [`VariadicFunc`].
pub const fn variadic_fn_name(func: VariadicFunc) -> &'static str {
    match func {
        VariadicFunc::Concat => "concat",
        VariadicFunc::ConcatWs => "concat_ws",
        VariadicFunc::Greatest => "greatest",
        VariadicFunc::Least => "least",
        VariadicFunc::NullIf => "nullif",
    }
}

/// Every [`VariadicFunc`], for the corpus completeness check. See
/// `pglet::dump`'s `ROUND_TRIP_CORPUS`.
pub const VARIADIC_FUNCS: [VariadicFunc; 5] = [
    VariadicFunc::Concat,
    VariadicFunc::ConcatWs,
    VariadicFunc::Greatest,
    VariadicFunc::Least,
    VariadicFunc::NullIf,
];

/// The SQL spelling of a [`StringFunc`], and an unaliased call's output column name.
///
/// `Expr::output_name` reads it for the name. [`StringFunc::Position`] has no function
/// spelling for the deparser (see [`render_string_fn`]); its `position` is the output
/// name alone.
///
/// `btrim` is emitted for [`StringFunc::Btrim`] even though `TRIM(…)` also builds
/// it, and `substring` for [`StringFunc::Substring`] even though the
/// `SUBSTRING(x FROM a FOR b)` grammar also does: in both cases the plain
/// comma-form call re-parses to the same variant on **both** engines, so the
/// special-form spelling buys nothing.
pub const fn string_fn_name(func: StringFunc) -> &'static str {
    match func {
        StringFunc::Upper => "upper",
        StringFunc::Length => "length",
        StringFunc::Replace => "replace",
        StringFunc::Substring => "substring",
        StringFunc::Ltrim => "ltrim",
        StringFunc::Rtrim => "rtrim",
        StringFunc::Btrim => "btrim",
        StringFunc::Strpos => "strpos",
        StringFunc::Lpad => "lpad",
        StringFunc::Rpad => "rpad",
        StringFunc::SplitPart => "split_part",
        StringFunc::DateTrunc => "date_trunc",
        StringFunc::CurrentSetting => "current_setting",
        StringFunc::CurrentSchemas => "current_schemas",
        StringFunc::PgGetConstraintdef => "pg_get_constraintdef",
        // Rendered by `render_string_fn` as the `::regclass` cast it was written as,
        // never by name — the reason is **pglet's own**, and one-sided. `SELECT
        // regclass('t')` is `0A000 function not supported: regclass` here (as `int4(1)`
        // and `text(1)` are: this engine has no function-style cast either), so a dump
        // emitting `regclass(x)` would not reload into pglet. 18.6 answers that call
        // spelling — `pg_proc` 1079 is `regclass(text)`, and `regclass(1)` is `1` there
        // through the `typename(expr)` coercion — so the reason this arm exists is a
        // property of pglet and not of both engines. `EDGES.md` row 121 is the ledger
        // record for that call form and the type names beside it.
        StringFunc::Regclass => "regclass",
        // Rendered by `render_string_fn`, never by name; `position` is the output column
        // name `Expr::output_name` reads here.
        StringFunc::Position => "position",
        StringFunc::IsFinite => "isfinite",
        StringFunc::ColDescription => "col_description",
        StringFunc::AdvisoryXactLock => "pg_advisory_xact_lock",
        StringFunc::QuoteIdent => "quote_ident",
        StringFunc::PgGetSerialSequence => "pg_get_serial_sequence",
        StringFunc::HasSchemaPrivilege => "has_schema_privilege",
        // Rendered by `render_string_fn` as the `::<enum>` cast it was written as.
        StringFunc::EnumIn => "enum_in",
        StringFunc::Md5 => "md5",
        StringFunc::Left => "left",
        StringFunc::Right => "right",
        StringFunc::Substr => "substr",
        StringFunc::OctetLength => "octet_length",
    }
}

/// Every [`StringFunc`], for the corpus completeness check. See
/// `pglet::dump`'s `ROUND_TRIP_CORPUS`.
pub const STRING_FUNCS: [StringFunc; 29] = [
    StringFunc::PgGetConstraintdef,
    StringFunc::Regclass,
    StringFunc::Upper,
    StringFunc::Length,
    StringFunc::Replace,
    StringFunc::Substring,
    StringFunc::Ltrim,
    StringFunc::Rtrim,
    StringFunc::Btrim,
    StringFunc::Strpos,
    StringFunc::Lpad,
    StringFunc::Rpad,
    StringFunc::SplitPart,
    StringFunc::DateTrunc,
    StringFunc::Position,
    StringFunc::CurrentSetting,
    StringFunc::CurrentSchemas,
    StringFunc::IsFinite,
    StringFunc::ColDescription,
    StringFunc::AdvisoryXactLock,
    StringFunc::QuoteIdent,
    StringFunc::PgGetSerialSequence,
    StringFunc::HasSchemaPrivilege,
    StringFunc::EnumIn,
    StringFunc::Md5,
    StringFunc::Left,
    StringFunc::Right,
    StringFunc::Substr,
    StringFunc::OctetLength,
];

/// Render a [`Expr::StringFn`] call, special-casing [`StringFunc::Position`].
///
/// `Position` is reachable **only** from the `POSITION(sub IN str)` special form —
/// `planner::build_call` has no `position` entry — so it must render back to that
/// form. `strpos(str, sub)` would evaluate identically but re-parse as
/// [`StringFunc::Strpos`], a different variant, which is exactly what the
/// round-trip invariant forbids.
///
/// **The argument order is the trap.** The parser stores `[haystack, needle]` for
/// *both* spellings, so the `IN` clause reads the arguments back-to-front:
/// `POSITION(args[1] IN args[0])`. Postgres does the same (`POSITION(a IN b)`
/// resolves to `position(text, text)` with the arguments swapped relative to the
/// clause), and deparses as `POSITION(('x'::text) IN (s))`.
fn render_string_fn(func: StringFunc, args: &[Expr], mode: RenderMode<'_>) -> Result<String> {
    if matches!(func, StringFunc::Position) {
        let [haystack, needle] = args else {
            return Err(Error::NotImplemented(format!(
                "cannot render POSITION with {} argument(s)",
                args.len()
            )));
        };
        let ty = matches!(mode, RenderMode::Catalog(_)).then_some(TypeOid::Text);
        return Ok(format!(
            "POSITION({} IN {})",
            render_operand(needle, ty, mode)?,
            render_operand(haystack, ty, mode)?
        ));
    }
    // `::regclass` is written as a cast and must deparse back as one: `SELECT
    // regclass('t')` is `0A000 function not supported: regclass` here, so a persisted
    // expression rendered as `regclass(x)` dumps to SQL **pglet** does not reload. 18.6
    // answers that spelling (measured), which is why the reason is stated one-sidedly.
    if matches!(func, StringFunc::Regclass) {
        let [inner] = args else {
            return Err(Error::NotImplemented(format!(
                "cannot render ::regclass with {} argument(s)",
                args.len()
            )));
        };
        return Ok(format!("({})::regclass", render_expr_mode(inner, mode)?));
    }
    // `::<enum>` is written as a cast and deparses as one, in both modes — 18.6's
    // `pg_get_expr` prints a `CoerceViaIO` to an enum as `(s)::mq`. A string-literal
    // operand is a constant the enum does not hold (a member never builds this node) and
    // is spelled `'q'::mq`, the form the parser reads back to the same node; no accepted
    // DDL persists one, the DDL walk refusing it.
    if matches!(func, StringFunc::EnumIn) {
        let Some((operand, name, _)) = crate::planner::enum_in_arg_parts(args) else {
            return Err(Error::NotImplemented(format!(
                "cannot render an enum cast with {} argument(s)",
                args.len()
            )));
        };
        let name = quote_ident(name);
        return Ok(match operand {
            Expr::Literal(lit @ Literal::String(_)) => {
                format!("{}::{name}", render_const(lit, None, mode))
            }
            other => format!("({})::{name}", render_expr_mode(other, mode)?),
        });
    }
    if let RenderMode::Catalog(_) = mode {
        let rendered: Result<Vec<String>> = args
            .iter()
            .enumerate()
            .map(|(i, a)| {
                let ty = string_fn_arg_text(func, i).then_some(TypeOid::Text);
                render_operand(a, ty, mode)
            })
            .collect();
        // `generate_function_name` runs the name through `quote_identifier`, so a
        // function whose name is a **keyword** comes back quoted: `substring(s, 1, 2)`
        // reads `"substring"(s, 1, 2)` on 18.6 (measured; `substring` is a
        // `col_name_keyword`), and `left(s, 1)` reads `"left"(s, 1)` (`left` and `right`
        // are `type_func_name_keyword`s). The same rule here, 18.6's own keyword table
        // (`lexer::pg_quote`). `position` never renders by name (it takes the
        // `POSITION(x IN y)` special form). The `SUBSTRING(s FROM 1 FOR 2)`
        // spelling of the same call is 18.6's `COERCE_SQL_SYNTAX` memory of how it was
        // *written*, a bit pglet does not store, so it stays recorded.
        let name = crate::lexer::pg_quote::quote_identifier(string_fn_name(func));
        return Ok(format!("{name}({})", rendered?.join(", ")));
    }
    render_call(string_fn_name(func), args, mode)
}

/// Whether an *unknown* literal at argument `idx` of `func` resolves to `text`, so
/// `upper('x')` deparses `upper('x'::text)` as it does on 18.6.
///
/// Where a name also has a `bytea` overload at that position (`length`, `substring`,
/// `substr`, `md5`, `octet_length`) the literal still resolves to `text`, the string
/// category's preferred type — measured over `md5('a')`, `substr('abc', 2)` and
/// `octet_length('abc')`, which read back with `'…'::text`.
///
/// Read off `pg_get_function_arguments` for each name on 18.6 rather than assumed: the
/// padding and splitting functions take an `integer` in the middle, `current_setting`
/// a `boolean` second argument, and `substring` / `date_trunc` are overloaded on their
/// later arguments — so those positions answer `false` and render as pglet always has.
/// `false` is always safe here; it means "no resolution", never a wrong one.
const fn string_fn_arg_text(func: StringFunc, idx: usize) -> bool {
    match func {
        StringFunc::Upper
        | StringFunc::Length
        | StringFunc::Ltrim
        | StringFunc::Rtrim
        | StringFunc::Btrim
        | StringFunc::Replace
        | StringFunc::Strpos
        | StringFunc::Position
        // every overload pglet models of these takes `text` in each place
        | StringFunc::QuoteIdent
        | StringFunc::PgGetSerialSequence
        | StringFunc::HasSchemaPrivilege
        | StringFunc::Md5
        | StringFunc::OctetLength => true,
        StringFunc::Lpad | StringFunc::Rpad => idx == 0 || idx == 2,
        StringFunc::SplitPart => idx < 2,
        StringFunc::Substring
        | StringFunc::Substr
        | StringFunc::Left
        | StringFunc::Right
        | StringFunc::DateTrunc
        | StringFunc::CurrentSetting => idx == 0,
        StringFunc::CurrentSchemas
        | StringFunc::Regclass
        | StringFunc::PgGetConstraintdef
        | StringFunc::IsFinite
        | StringFunc::ColDescription
        | StringFunc::AdvisoryXactLock
        | StringFunc::EnumIn => false,
    }
}

/// The SQL spelling of a niladic [`BuiltinFn`].
///
/// **Asymmetric on purpose.** `now()`, `gen_random_uuid()` and the three
/// `*_timestamp()` members are ordinary functions, while the other five are Postgres
/// *SQL value functions* — parenless special forms (`func_expr_common_subexpr` in
/// `gram.y`) — and both engines reject the parenthesized spelling (`current_date()`
/// is `42601`). A uniform `name()` renderer would break five of the ten.
const fn builtin_sql(func: BuiltinFn) -> &'static str {
    match func {
        BuiltinFn::Now => "now()",
        BuiltinFn::TransactionTimestamp => "transaction_timestamp()",
        BuiltinFn::StatementTimestamp => "statement_timestamp()",
        BuiltinFn::ClockTimestamp => "clock_timestamp()",
        // A SQL value function like the four below it, not a call.
        BuiltinFn::CurrentTimestamp => "CURRENT_TIMESTAMP",
        BuiltinFn::RandomUuid => "gen_random_uuid()",
        BuiltinFn::CurrentDate => "CURRENT_DATE",
        BuiltinFn::CurrentTime => "CURRENT_TIME",
        BuiltinFn::LocalTime => "LOCALTIME",
        BuiltinFn::LocalTimestamp => "LOCALTIMESTAMP",
        // The role forms and `CURRENT_CATALOG` / `CURRENT_SCHEMA` are parenless
        // keywords too; `pg_get_expr` renders each stored `DEFAULT` back as the
        // **written** keyword, upper-cased — measured on 18.6, all six.
        BuiltinFn::CurrentUser => "CURRENT_USER",
        BuiltinFn::SessionUser => "SESSION_USER",
        BuiltinFn::User => "USER",
        BuiltinFn::CurrentRole => "CURRENT_ROLE",
        BuiltinFn::CurrentCatalog => "CURRENT_CATALOG",
        BuiltinFn::CurrentSchema => "CURRENT_SCHEMA",
        // ...and the two call spellings. `current_schema()` renders **quoted** on
        // 18.6 (`"current_schema"()`), because the bare word would re-lex as the
        // keyword and mean the parenless form instead — the one place in this table
        // where the quoting is load-bearing rather than cosmetic.
        BuiltinFn::CurrentSchemaFn => "\"current_schema\"()",
        BuiltinFn::CurrentDatabase => "current_database()",
        BuiltinFn::Version => "version()",
        BuiltinFn::Random => "random()",
    }
}

/// Every [`BuiltinFn`], for the corpus completeness check. See
/// `pglet::dump`'s `ROUND_TRIP_CORPUS`.
pub const BUILTIN_FNS: [BuiltinFn; 20] = [
    BuiltinFn::Now,
    BuiltinFn::RandomUuid,
    BuiltinFn::CurrentDate,
    BuiltinFn::CurrentTime,
    BuiltinFn::LocalTime,
    BuiltinFn::LocalTimestamp,
    BuiltinFn::TransactionTimestamp,
    BuiltinFn::StatementTimestamp,
    BuiltinFn::ClockTimestamp,
    BuiltinFn::CurrentTimestamp,
    BuiltinFn::CurrentUser,
    BuiltinFn::SessionUser,
    BuiltinFn::User,
    BuiltinFn::CurrentRole,
    BuiltinFn::CurrentCatalog,
    BuiltinFn::CurrentDatabase,
    BuiltinFn::CurrentSchema,
    BuiltinFn::CurrentSchemaFn,
    BuiltinFn::Version,
    BuiltinFn::Random,
];

/// Render an [`Expr::Extract`] — `EXTRACT(field FROM source)` / `date_part(field,
/// source)`.
///
/// `retnumeric` is **not** cosmetic: it is the whole difference between
/// `EXTRACT`/`extract` (returns `numeric`) and `date_part` (returns `double
/// precision`), which Postgres has kept distinct since 14. Substituting one for the
/// other is a type change, so each renders to its own spelling.
///
/// The `retnumeric` (EXTRACT) side has two shapes:
///
/// - **A text-literal field** — `EXTRACT('year' FROM ts)`, using Postgres's
///   `extract_arg: … | Sconst` extension. Both engines parse it, and pglet folds
///   the bare-word form (`EXTRACT(year FROM ts)`) to the same string literal, so
///   quoting is the safe universal spelling (no keyword can collide).
/// - **Any other field expression** — the **sanctioned fallback**. pglet accepts a
///   comma call with an arbitrary field expression (`extract(lower('YEAR'), ts)`)
///   and stores it; Postgres's `extract_list` grammar is `extract_arg FROM a_expr`
///   and cannot spell one at all (`extract(lower('YEAR'), ts)` is a syntax error
///   there, verified against 18.6). So this renders back to pglet's comma form: it
///   round-trips to the identical variant *here*, and the cost is that such a dump
///   does not reload into stock Postgres at that one statement. Declining instead
///   would lose the whole database's dump, and the divergence is pglet's grammar
///   being wider, not the renderer's.
fn render_extract(
    source: &Expr,
    field: &Expr,
    retnumeric: bool,
    mode: RenderMode<'_>,
) -> Result<String> {
    let src = render_expr_mode(source, mode)?;
    match extract_spelling(field, retnumeric) {
        // `date_part(text, …)` declares `text` in every `pg_proc` overload, so an
        // unknown field literal resolves there and 18.6 stores the typed `Const`:
        // `date_part('year', t)` reads `date_part('year'::text, t)` (measured).
        ExtractSpelling::DatePart => Ok(format!(
            "date_part({}, {src})",
            render_operand(
                field,
                matches!(mode, RenderMode::Catalog(_)).then_some(TypeOid::Text),
                mode
            )?
        )),
        // 18.6 runs the field `Const`'s text through `quote_identifier`
        // (`ruleutils.c`'s `F_EXTRACT_TEXT_*` arm), so the written case survives as a
        // quoted identifier: `EXTRACT('YEAR' FROM t)` reads back
        // `EXTRACT("YEAR" FROM t)`, `EXTRACT('select' FROM t)` `EXTRACT("select" FROM
        // t)`, and a lowercase unit word stays bare — `EXTRACT(year FROM t)`. The
        // stored string is the one 18.6 stores (the parser folds a bare field word
        // and keeps a quoted one or a string constant verbatim), so the same rule over
        // it is the same answer. `Portable` keeps the quoted literal — it is what
        // pglet's own parser reads back to this variant whatever the string holds.
        ExtractSpelling::ExtractFrom(name) => Ok(match mode {
            RenderMode::Catalog(_) => format!(
                "EXTRACT({} FROM {src})",
                crate::lexer::pg_quote::quote_identifier(name)
            ),
            RenderMode::Portable => {
                format!("EXTRACT({} FROM {src})", quote_string_literal(name))
            }
        }),
        ExtractSpelling::ExtractComma => Ok(format!(
            "extract({}, {src})",
            render_expr_mode(field, mode)?
        )),
    }
}

/// Which of [`render_extract`]'s three spellings an [`Expr::Extract`] takes.
///
/// A named choice rather than an inline `if` chain because the render arm and the
/// **test-side coverage discriminant** must agree: coverage keyed on the bare
/// `Extract` tag let one corpus entry stand in for all three arms, so the
/// discriminant folds this selection in — and reading it from the same function the
/// renderer dispatches on is what keeps the two from drifting.
///
/// [`Self::ExtractFrom`] carries the literal it matched, so the caller needs no
/// second (fallible) destructure to reach it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtractSpelling<'a> {
    /// `date_part(field, source)` — the `float8`-returning twin (`retnumeric: false`).
    DatePart,
    /// `EXTRACT('field' FROM source)` — the SQL special form, available only because
    /// the field is a text literal Postgres's `extract_arg` grammar can spell.
    ExtractFrom(&'a str),
    /// `extract(field, source)` — the sanctioned pglet-only fallback for a
    /// `retnumeric` node whose field is *not* a literal.
    ExtractComma,
}

/// The spelling [`render_extract`] must use for this `(field, retnumeric)` pair.
pub fn extract_spelling(field: &Expr, retnumeric: bool) -> ExtractSpelling<'_> {
    match (retnumeric, field) {
        (false, _) => ExtractSpelling::DatePart,
        (true, Expr::Literal(Literal::String(name))) => ExtractSpelling::ExtractFrom(name),
        (true, _) => ExtractSpelling::ExtractComma,
    }
}

/// `(<left> <op> ANY (<right>))` / `… ALL (…)` — an [`Expr::AnyAll`] quantified
/// array comparison.
///
/// In [`RenderMode::Portable`] this does not collide with the [`Expr::InList`] arm even
/// though Postgres deparses `x IN (1, 2)` and `x = ANY (ARRAY[1, 2])` to byte-identical
/// text: the two are distinct persisted variants and each renders its own source
/// spelling, so both re-parse to what they were. In [`RenderMode::Catalog`] they **do**
/// collide, and must — 18.6 stores one node for both spellings, so a rendering that
/// kept them apart would report one of them wrong. See [`render_in_list`].
///
/// The parser only builds `AnyAll` over a comparison operator, all six of which
/// [`binary_op_sql`] spells; the `None` arm is the can't-happen for a
/// function-modeled operator (`starts_with`/`to_char`), which has no `ANY` form in
/// either engine.
#[inline(never)]
fn render_any_all(
    left: &str,
    op: BinaryOp,
    right: &Expr,
    all: bool,
    mode: RenderMode<'_>,
) -> Result<String> {
    let Some(sym) = binary_op_sql(op) else {
        return Err(Error::NotImplemented(format!(
            "cannot render {op:?} as a quantified ANY/ALL comparison"
        )));
    };
    let quantifier = if all { "ALL" } else { "ANY" };
    // The right operand is rendered with **no** element type pushed down, so its
    // `ARRAY[…]` falls to [`common_element_type`] over its own members. That is
    // deliberate, and it is not the left operand's type: measured on 18.6,
    // `s = ANY (ARRAY['x','y'])` over `text` reads `ARRAY['x'::text, 'y'::text]`, but
    // over a `varchar(10)` column it reads the **same** `ARRAY['x'::text, …]` with the
    // *column* relabelled, and over a `numeric` column with `int` members it casts the
    // whole array (`ANY ((ARRAY[1, 2])::numeric[])`) rather than its elements. So
    // deriving the element type from the left operand would be right in the one case
    // the members already answer and wrong in the two they do not.
    Ok(format!(
        "({left} {sym} {quantifier} ({}))",
        render_expr_mode(right, mode)?
    ))
}

/// Decline to render an expression form, naming it.
///
/// Returns `Err` unconditionally; the `Ok`-typed return lets a `match` arm in
/// [`render_expr`] call it with `?` and stay in the arm's `String` type.
///
/// # Errors
///
/// Always — [`Error::NotImplemented`].
fn unrenderable(what: &str) -> Result<String> {
    Err(Error::NotImplemented(format!(
        "cannot render expression to portable SQL: {what}"
    )))
}

/// Render a `CASE … WHEN … THEN … [ELSE …] END`.
fn render_case(
    operand: Option<&Expr>,
    when_clauses: &[(Expr, Expr)],
    else_result: Option<&Expr>,
    mode: RenderMode<'_>,
) -> Result<String> {
    // A `CASE`'s branch results are a `select_common_type` position like `COALESCE`'s
    // arguments, so an *unknown* literal there takes the resolved type and 18.6 stores
    // the typed `Const`: `CASE WHEN (a = 1) THEN 'y' ELSE 'n' END` reads
    // `THEN 'y'::text … ELSE 'n'::text` (measured, beside the `int` branches that stay
    // bare). The **test-expression** form resolves its `WHEN` values against the
    // operand the same way. Both decline when two known members disagree, which is the
    // coercion set this renderer does not model. The four-line
    // `PRETTYFLAG_INDENT` layout stays recorded (`EDGES.md` row 91); this is the
    // constants' rule, not the formatter's.
    let (test_ty, result_ty) = match mode {
        RenderMode::Portable => (None, None),
        RenderMode::Catalog(scope) => {
            let test_ty = operand.map(|op| {
                let mut members: Vec<Expr> = vec![op.clone()];
                members.extend(when_clauses.iter().map(|(cond, _)| cond.clone()));
                common_element_type(&members, scope)
            });
            let mut results: Vec<Expr> = when_clauses.iter().map(|(_, r)| r.clone()).collect();
            if let Some(e) = else_result {
                results.push(e.clone());
            }
            (test_ty.flatten(), common_element_type(&results, scope))
        }
    };
    let mut out = String::from("CASE");
    if let Some(op) = operand {
        let _ = write!(out, " {}", render_expr_mode(op, mode)?);
    }
    for (cond, result) in when_clauses {
        // A conditionless `CASE`'s `WHEN` is a boolean predicate, not a
        // common-type position — `test_ty` is `None` there and `render_operand`
        // leaves the operand alone.
        let _ = write!(
            out,
            " WHEN {} THEN {}",
            render_operand(cond, test_ty, mode)?,
            render_operand(result, result_ty, mode)?
        );
    }
    if let Some(e) = else_result {
        let _ = write!(out, " ELSE {}", render_operand(e, result_ty, mode)?);
    } else if let (RenderMode::Catalog(_), Some(ty)) = (mode, result_ty) {
        // **18.6 always stores an `ELSE`**: `transformCaseExpr` supplies a NULL
        // `defresult` when none is written, and `get_const_expr` labels a NULL whose
        // type is known — so `CASE WHEN (a = 1) THEN 1 END` reads
        // `… ELSE NULL::integer END` there (measured at five result types, including
        // the branch-typed and the unknown-resolved ones). `Portable` must **not** add
        // it: `parse(render(e)) == e` is the identical variant, and an emitted
        // `ELSE NULL` re-parses to `else_result: Some(..)`. A result type that does not
        // resolve emits nothing rather than an unlabelled `ELSE NULL`, this renderer's
        // standing decline.
        if let Ok(name) = catalog_type_name(ty) {
            let _ = write!(out, " ELSE NULL::{name}");
        }
    }
    out.push_str(" END");
    Ok(out)
}

/// Render a binary operation: an infix operator (`(l op r)`), or the function form
/// for the function-modeled ops (`starts_with`, `to_char`, `mod`, `power`, `pow`).
///
/// `AND` / `OR` are the exception, and they take [`render_bool_chain`] — 18.6's node for
/// them is **N-ary**, not binary.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
fn render_binary_op(
    left: &Expr,
    op: BinaryOp,
    right: &Expr,
    mode: RenderMode<'_>,
) -> Result<String> {
    // The operands are rendered from this frame, and everything else decided and spelled
    // in frames of its own, so an operator chain costs this frame a level
    // (`crate::depth`).
    if let Some(rendered) = render_binary_whole(left, op, right, mode)? {
        return Ok(rendered);
    }
    let (want_l, want_r) = binary_operand_wants(left, op, right, mode);
    let l = render_operand(left, want_l, mode)?;
    let r = render_operand(right, want_r, mode)?;
    render_binary_tail(op, &l, &r, mode)
}

/// The binary nodes rendered whole rather than as `(l op r)`: an `AND`/`OR` list, and
/// an `IS [NOT] DISTINCT FROM` against a bare `NULL` in a catalog rendering.
#[inline(never)]
fn render_binary_whole(
    left: &Expr,
    op: BinaryOp,
    right: &Expr,
    mode: RenderMode<'_>,
) -> Result<Option<String>> {
    if matches!(op.connective(), BinaryOp::And | BinaryOp::Or) {
        return render_bool_chain(left, op, right, mode).map(Some);
    }
    // **An `IS [NOT] DISTINCT FROM` against an undecorated `NULL` is not a
    // `DistinctExpr` on 18.6 at all** — `transformAExprDistinct` short-circuits to
    // `make_nulltest_from_distinct` when either operand is a bare NULL `A_Const`, so
    // `CHECK (a IS NOT DISTINCT FROM NULL)` is stored as a `NullTest` and reads back
    // `CHECK ((a IS NULL))`, and the `DISTINCT` polarity reads `(a IS NOT NULL)`
    // (`src/backend/parser/parse_expr.c` in the PostgreSQL source; both polarities, both operand
    // sides and a computed operand measured on 18.6). Three properties of that
    // function are load-bearing here and each is measured:
    //
    // - **Bare only.** `exprIsNullConstant` tests for an `A_Const` carrying `isnull`,
    //   so a *cast* NULL is a `TypeCast` node and keeps the `DistinctExpr` spelling:
    //   `a IS NOT DISTINCT FROM NULL::int` reads `(NOT (a IS DISTINCT FROM
    //   NULL::integer))`. pglet's parser draws the same line — a bare `NULL` (and a
    //   parenthesized `(NULL)`) stays `Expr::Literal(Literal::Null)` while `NULL::int`
    //   is an `Expr::Cast`, confirmed through the `Portable` dump.
    // - **The right operand is tested first**, so `NULL IS NOT DISTINCT FROM NULL`
    //   keeps the *left* NULL as the tested argument. 18.6 prints it `(NULL::unknown
    //   IS NULL)` — pglet models no `unknown` type name and no position resolves this
    //   one, so it renders the bare `(NULL IS NULL)`: the family's one residual, and
    //   the only member of the eighteen shapes measured that does not agree.
    // - **The argument is not coerced** ("the argument can be any type, so don't coerce
    //   it"), so no type is pushed into it: `'x' IS NOT DISTINCT FROM NULL` reads
    //   `('x' IS NULL)`, the literal unlabelled.
    //
    // `Portable` keeps the written keyword — it is what pglet's own parser reads back
    // to the same variant, which the dump's round-trip invariant needs.
    if let (BinaryOp::IsDistinctFrom | BinaryOp::IsNotDistinctFrom, RenderMode::Catalog(_)) =
        (op, mode)
    {
        let bare_null = |e: &Expr| matches!(e, Expr::Literal(Literal::Null));
        let arg = if bare_null(right) {
            Some(left)
        } else if bare_null(left) {
            Some(right)
        } else {
            None
        };
        if let Some(arg) = arg {
            let rendered = render_expr_mode(arg, mode)?;
            let test = if matches!(op, BinaryOp::IsNotDistinctFrom) {
                "IS NULL"
            } else {
                "IS NOT NULL"
            };
            return Ok(Some(format!("({rendered} {test})")));
        }
    }
    Ok(None)
}

/// The type an *unknown* literal resolves to at each operand position of `op`, in a
/// catalog rendering.
#[inline(never)]
fn binary_operand_wants(
    left: &Expr,
    op: BinaryOp,
    right: &Expr,
    mode: RenderMode<'_>,
) -> (Option<TypeOid>, Option<TypeOid>) {
    // In a catalog rendering each operand is told what an *unknown* literal in its
    // position resolved to, which is the whole of `(s = 'x'::text)` against
    // `(s = 'x')`. `operand_type` types a string literal as `unknown`, so a sibling
    // that is itself one contributes no answer and `unknown_resolves_to` supplies the
    // all-unknown default for the operators that have one.
    match mode {
        RenderMode::Portable => (None, None),
        RenderMode::Catalog(scope) => {
            let lt = operand_type(left, scope);
            let rt = operand_type(right, scope);
            (unknown_resolves_to(op, rt), unknown_resolves_to(op, lt))
        }
    }
}

/// `op` spelled between its two rendered operands.
#[inline(never)]
fn render_binary_tail(op: BinaryOp, l: &str, r: &str, mode: RenderMode<'_>) -> Result<String> {
    // **18.6 has no `IS NOT DISTINCT FROM` node.** `transformAExprDistinct` builds the
    // `DistinctExpr` and wraps it in a `NOT` for the negated spelling, so
    // `CHECK (a IS NOT DISTINCT FROM 1)` reads back `CHECK ((NOT (a IS DISTINCT FROM
    // 1)))` there (measured, beside the agreeing positive polarity). `Portable` keeps
    // the written keyword: it is the spelling pglet's own parser reads back to the same
    // variant, which the dump's round-trip invariant needs.
    if let (BinaryOp::IsNotDistinctFrom, RenderMode::Catalog(_)) = (op, mode) {
        return Ok(format!("(NOT ({l} IS DISTINCT FROM {r}))"));
    }
    Ok(match binary_op_sql_mode(op, mode) {
        Some(sym) => format!("({l} {sym} {r})"),
        None => match op {
            BinaryOp::StartsWith => format!("starts_with({l}, {r})"),
            BinaryOp::ToChar => format!("to_char({l}, {r})"),
            // `mod(…)` / `power(…)` / `pow(…)` under the name written, the spelling
            // 18.6 prints them back in and pglet's parser reads to the same variant.
            BinaryOp::ModFn | BinaryOp::PowerFn | BinaryOp::PowFn => {
                format!("{}({l}, {r})", op.call_name().unwrap_or("mod"))
            }
            // `binary_op_sql` covers every other variant, so this is unreachable;
            // fail loud rather than panic if a new op is added without a spelling.
            _ => {
                return Err(Error::NotImplemented(format!(
                    "cannot render binary operator {op:?} to SQL"
                )));
            }
        },
    })
}

/// Render an `AND` / `OR` chain the way `get_rule_expr`'s `BoolExpr` arm does: one
/// parenthesized list of **N** operands, not a nest of binary nodes.
///
/// 18.6's `BoolExpr` carries a list of args, and `makeAndExpr` / `makeOrExpr` append to
/// that list *on sight* while parsing a left-deep chain — looking through parentheses as
/// they go, so `(a AND b) AND c` flattens exactly as `a AND b AND c` does, while
/// `a AND (b AND c)` does not (its left operand is not a `BoolExpr`, so the nested one
/// becomes an ordinary argument). pglet's parser is left-associative and drops
/// parentheses, so the **left spine** of same-operator nodes is that same arg list — and
/// a list longer than `Expr::bool_list`'s left-deep bound is built balanced under a
/// [`BinaryOp::AndList`] root, whose [`BinaryOp::AndListInner`] nodes are the list's own
/// and whose other leaves — a parenthesized long list among them — are its operands —
/// and walking them reproduces 18.6's text at every operand count and nesting shape
/// **measured below** —
/// which is the claim this carries, the general one not being derivable from a reading
/// of one deparser arm.
///
/// Measured against `pg_get_constraintdef` on 18.6 over eighteen CHECK constraints — the
/// three-, four- and two-conjunct chains, both paren spellings of a three-conjunct one,
/// the four mixed `AND`/`OR` nestings, and the `NOT` shapes. Without the flatten only the
/// **two**-operand arity agreed, which is the arity every fixture pinning this renderer
/// used.
///
/// Only `AND` / `OR` flatten: 18.6 renders `a + b + c` as `((a + b) + c)`, so the
/// arithmetic operators keep their binary nesting.
fn render_bool_chain(
    left: &Expr,
    op: BinaryOp,
    right: &Expr,
    mode: RenderMode<'_>,
) -> Result<String> {
    // The list's operands in source order: a plain chain's left spine, or the leaves
    // under a long list's root — never into an operand the user parenthesized in a
    // later position.
    let mut operands: Vec<&Expr> = Vec::new();
    Expr::bool_node_operands(left, op, right, &mut operands);
    let rendered = operands
        .iter()
        .map(|e| render_expr_mode(e, mode))
        .collect::<Result<Vec<_>>>()?;
    // `binary_op_sql` is total for `And`/`Or`; the fallback keeps this arm from needing a
    // second unreachable-error path.
    let sym = binary_op_sql(op).unwrap_or("AND");
    Ok(format!("({})", rendered.join(&format!(" {sym} "))))
}

/// The infix spelling in this rendering's own dialect.
///
/// The four pattern operators are the only ones that differ, and they differ because
/// `LIKE` is *grammar* rather than an operator name: the parser builds `~~` / `!~~` /
/// `~~*` / `!~~*` and `ruleutils.c` prints the operator, so `CHECK (s LIKE 'x%')` reads
/// back `CHECK ((s ~~ 'x%'::text))` on 18.6 (measured, all four). A dump keeps the
/// keyword spelling — pglet's parser has no `~~` operator, so a dump emitting one would
/// not reload.
const fn binary_op_sql_mode(op: BinaryOp, mode: RenderMode<'_>) -> Option<&'static str> {
    if let RenderMode::Catalog(_) = mode {
        match op {
            BinaryOp::Like => return Some("~~"),
            BinaryOp::NotLike => return Some("!~~"),
            BinaryOp::Ilike => return Some("~~*"),
            BinaryOp::NotIlike => return Some("!~~*"),
            _ => {}
        }
    }
    binary_op_sql(op)
}

/// The infix SQL spelling of a binary operator, or `None` for the function-modeled
/// `starts_with` / `to_char` and the call-spelled `mod` / `power` / `pow`.
pub const fn binary_op_sql(op: BinaryOp) -> Option<&'static str> {
    Some(match op {
        BinaryOp::Eq => "=",
        BinaryOp::Ne => "<>",
        BinaryOp::Lt => "<",
        BinaryOp::Le => "<=",
        BinaryOp::Gt => ">",
        BinaryOp::Ge => ">=",
        BinaryOp::And | BinaryOp::AndList | BinaryOp::AndListInner => "AND",
        BinaryOp::Or | BinaryOp::OrList | BinaryOp::OrListInner => "OR",
        BinaryOp::Add => "+",
        BinaryOp::Sub => "-",
        BinaryOp::Mul => "*",
        BinaryOp::Div => "/",
        BinaryOp::Mod => "%",
        BinaryOp::Pow => "^",
        BinaryOp::Concat => "||",
        BinaryOp::Like => "LIKE",
        BinaryOp::NotLike => "NOT LIKE",
        BinaryOp::Ilike => "ILIKE",
        BinaryOp::NotIlike => "NOT ILIKE",
        BinaryOp::RegexMatch => "~",
        BinaryOp::NotRegexMatch => "!~",
        BinaryOp::JsonGet => "->",
        BinaryOp::JsonGetText => "->>",
        BinaryOp::JsonExists => "?",
        BinaryOp::JsonExistsAny => "?|",
        BinaryOp::JsonExistsAll => "?&",
        BinaryOp::JsonPathGet => "#>",
        BinaryOp::JsonPathGetText => "#>>",
        BinaryOp::Containment => "@>",
        BinaryOp::ContainedBy => "<@",
        BinaryOp::AtTimeZone => "AT TIME ZONE",
        BinaryOp::IsDistinctFrom => "IS DISTINCT FROM",
        BinaryOp::IsNotDistinctFrom => "IS NOT DISTINCT FROM",
        BinaryOp::StartsWith
        | BinaryOp::ToChar
        | BinaryOp::ModFn
        | BinaryOp::PowerFn
        | BinaryOp::PowFn => return None,
    })
}

/// Render a literal in an expression context (booleans as `TRUE`/`FALSE`, strings
/// single-quoted).
pub fn render_literal(lit: &Literal) -> String {
    match lit {
        Literal::Null => "NULL".to_string(),
        Literal::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Literal::Int(n) => n.to_string(),
        // **Unreachable from SQL text**, and therefore uncoverable by
        // `ROUND_TRIP_CORPUS`: `parser::expr::number_literal` builds `Int` or
        // `Numeric` and it is the only site turning a number token into a literal, so no parse
        // produces a `Literal::Float` and no stored expression can hold one. The
        // variant *is* constructed elsewhere in the workspace — the planner's
        // `#[cfg(test)]` `Expr` serialization corpus — so the claim is unreachability
        // from SQL text, not that the type is never built. Left rendering rather than declining
        // because the arm costs nothing and the variant is still a legal IR value.
        //
        // Classified, not fixed: `f64::to_string` spells the non-finite values `inf`
        // / `-inf` / `NaN`, none of which re-parses as a numeric literal on either
        // engine (Postgres wants `'Infinity'::float8`). That is a latent round-trip
        // hazard the moment anything starts producing this variant — closing it means
        // choosing the quoted-cast spelling, which is a change to make alongside
        // whatever makes it reachable, not before.
        Literal::Float(f) => f.to_string(),
        Literal::String(s) => quote_string_literal(s),
        // A numeric literal dumps as its canonical decimal (a valid bare NUMERIC
        // constant that re-parses on both engines).
        Literal::Numeric(n) => n.to_display_string(),
    }
}
// =============================================================================
// Identifier + literal quoting
// =============================================================================

/// A **type** name position — an enum type reference, or a `CREATE TYPE` name.
///
/// The type-name parser reads one as a bare identifier and rejects a quoted type
/// name, so an enum name that isn't a safe bare identifier can't round-trip and
/// fails loud.
pub fn bare_type_ident(name: &str) -> Result<String> {
    if is_safe_bare_ident(name) {
        Ok(name.to_string())
    } else {
        Err(Error::NotImplemented(format!(
            "cannot emit type name {name:?} as a bare identifier for a dump"
        )))
    }
}

/// Quote a relation name for a **string-literal** name position — the `regclass`
/// argument of `nextval(…)` and `setval(…)` — as `quote_ident` inside
/// `quote_string_literal`.
///
/// The two quotings are not interchangeable and neither substitutes for the other. A
/// literal is read back by `SplitIdentifierString`, which downcases an unquoted part
/// and rejects interior whitespace, so a name that is not already a safe bare
/// identifier must carry its **identifier** quoting *inside* the string: `Odd_a_seq`
/// resolves to `odd_a_seq` when written bare and to `Odd_a_seq` when written
/// `"Odd_a_seq"`, and `s x` is `invalid name syntax` bare. The outer
/// `quote_string_literal` then escapes whatever that produced for the SQL string
/// grammar, so an internal `'` doubles and a backslash forces the `E'…'` form. pg_dump
/// spells it exactly this way — `nextval('public.plain_a_seq'::regclass)` bare and
/// `nextval('public."Odd_a_seq"'::regclass)` quoted, from the same `fmtId` predicate —
/// which is why the rule is `quote_ident`'s rather than a second "needs quoting"
/// predicate minted here: the identifier positions naming the same sequence
/// (`CREATE SEQUENCE`, `ALTER SEQUENCE … OWNED BY`, `SEQUENCE NAME`) go through
/// `quote_ident`, and a literal that disagrees with them names a different relation.
pub fn quote_name_literal(name: &str) -> String {
    quote_string_literal(&quote_ident(name))
}

/// Quote a string as a SQL single-quoted literal, doubling any internal `'`.
pub fn quote_string_literal(s: &str) -> String {
    // No backslash: a plain `'…'` with `''`-doubling is identical under any
    // `standard_conforming_strings` setting. A backslash is the one byte whose
    // meaning depends on that GUC, so emit those as an `E'…'` escape string
    // (backslash + quote doubled) — pglet's lexer and modern Postgres both decode
    // `E'…'` the same, so it round-trips regardless of the session GUC, with no SET
    // preamble needed.
    if s.contains('\\') {
        format!("E'{}'", s.replace('\\', "\\\\").replace('\'', "''"))
    } else {
        format!("'{}'", s.replace('\'', "''"))
    }
}

// =============================================================================
// The catalog rendering — 18.6's `pg_get_expr` over the tree 18.6 would have stored
// =============================================================================

/// The type name a `::` cast suffix takes in a catalog rendering — `format_type`'s
/// spelling, which is **not** the DDL spelling [`type_oid_sql`] emits.
///
/// The two disagree on four scalars (`character varying` / `timestamp without time
/// zone` / `timestamp with time zone` / `time without time zone` against `varchar` /
/// `timestamp` / `timestamptz` / `time`), measured against `format_type` on 18.6 for
/// every OID pglet models. A dump must keep the DDL spelling — it has to re-parse —
/// and a catalog view must keep this one, so the two names are two functions.
///
/// **It is total** — every `TypeOid` has a catalog spelling, `name` and `name[]`
/// included: `name` is one of the seven arms this function answers itself rather than
/// delegating, and `name[]` comes back through the array recursion above them, as every
/// array type does. The `Result` is the shape
/// [`type_oid_sql`]'s delegation needs and not a reachable refusal: `name` / `name[]` are
/// spellable, by the arms below. `catalog_type_name_is_total` walks
/// every OID and pins it.
///
/// # Errors
///
/// None reachable — see above. `Result` is kept so a `TypeOid` added without a
/// spelling is a loud [`unsupported_type`] out of [`type_oid_sql`] rather than a
/// panic here.
pub fn catalog_type_name(oid: TypeOid) -> Result<String> {
    if let Some(elem) = oid.array_element() {
        return Ok(format!("{}[]", catalog_type_name(elem)?));
    }
    Ok(match oid {
        TypeOid::Varchar => "character varying".to_string(),
        TypeOid::Timestamp => "timestamp without time zone".to_string(),
        TypeOid::Timestamptz => "timestamp with time zone".to_string(),
        TypeOid::Time => "time without time zone".to_string(),
        TypeOid::Char => "\"char\"".to_string(),
        TypeOid::Name => "name".to_string(),
        TypeOid::Oid => "oid".to_string(),
        TypeOid::Void => "void".to_string(),
        other => type_oid_sql(other)?,
    })
}

/// The type an operand carries **with a string literal and a bare `NULL` typed
/// `unknown`** (`None`), which is what `transformExpr` sees before it resolves them.
///
/// This is the one thing `Expr::infer_type` cannot answer: it types every
/// `Literal::String` as `text`, which is right for pglet's evaluator and wrong for
/// reproducing 18.6's rendering — `d = '2024-01-01'` deparses `'2024-01-01'::date`
/// there precisely because the literal was *not* text when the operator was resolved.
/// So this special-cases the two unknown-producing literals and delegates everything
/// else to [`Expr::infer_type_with`], supplying itself as that walk's operand hook so
/// the unknown-ness propagates up.
///
/// `None` is always safe here: it means "no resolution", and every caller's fallback
/// for `None` is the unlabelled spelling — pglet's own, and never an invented cast.
fn operand_type(expr: &Expr, scope: &dyn LeafScope) -> Option<TypeOid> {
    match expr {
        Expr::Literal(Literal::String(_) | Literal::Null) => None,
        Expr::Literal(lit) => lit.infer_type(),
        other => other.infer_type_with(scope, &|e| operand_type(e, scope)),
    }
}

/// The type an *unknown* operand of `op` resolves to, given the sibling operand's type.
///
/// Postgres resolves an unknown literal through operator resolution
/// (`oper_select_candidate` / `select_common_type`), which needs `pg_operator`. This
/// answers only for the operator families where the answer does not: the ones whose
/// chosen operator takes the **same** type on both sides (so an unknown takes the
/// sibling's), and the ones whose operand type is fixed at `text` whatever the sibling
/// is. Every other operator answers `None` and renders as pglet always has.
///
/// `Varchar` maps to `Text`, matching 18.6: `v = 'x'` over a `varchar(10)` column
/// deparses `((v)::text = 'x'::text)` — the literal is *text*, and it is the column
/// that gets relabelled (a relabel pglet does not store and does not render, recorded
/// in `EDGES.md`).
fn unknown_resolves_to(op: BinaryOp, sibling: Option<TypeOid>) -> Option<TypeOid> {
    let sibling = match sibling {
        Some(TypeOid::Varchar) => Some(TypeOid::Text),
        other => other,
    };
    match op {
        // Same-type operators: the unknown takes the sibling's type, and a pair of
        // unknowns takes `text` (`select_common_type`'s all-unknown rule).
        BinaryOp::Eq
        | BinaryOp::Ne
        | BinaryOp::Lt
        | BinaryOp::Le
        | BinaryOp::Gt
        | BinaryOp::Ge
        | BinaryOp::IsDistinctFrom
        | BinaryOp::IsNotDistinctFrom
        | BinaryOp::Containment
        | BinaryOp::ContainedBy => Some(sibling.unwrap_or(TypeOid::Text)),
        // Pattern matching is `text ~~ text` whatever the left operand's type, and the regex
        // operators' pattern is `text` too (`CHECK (s ~ '^x')` reads back `(s ~ '^x'::text)`
        // on 18.6, measured). The jsonb key/path accessors take a `text` key on the right;
        // the left is never unknown in a stored expression (it is the jsonb column).
        BinaryOp::Like
        | BinaryOp::NotLike
        | BinaryOp::Ilike
        | BinaryOp::NotIlike
        | BinaryOp::RegexMatch
        | BinaryOp::NotRegexMatch
        | BinaryOp::JsonGet
        | BinaryOp::JsonGetText
        | BinaryOp::JsonExists => Some(TypeOid::Text),
        // `||` is `text || text` when either side is unknown *and* the other is a
        // string; over an array or a jsonb sibling Postgres picks a different
        // operator, so decline rather than guess.
        BinaryOp::Concat => match sibling {
            None | Some(TypeOid::Text) => Some(TypeOid::Text),
            _ => None,
        },
        _ => None,
    }
}

/// Whether an expression contains a column reference — `transformAExprIn`'s
/// `contain_vars_of_level` test, which decides whether an `IN` element joins the
/// `ScalarArrayOpExpr` or stays an `OR`/`AND` operand of its own.
fn contains_column(expr: &Expr) -> bool {
    expr.any_node(&|node| matches!(node, Expr::Column { .. }).then_some(true))
}

/// Whether a numeric text is one `get_const_expr` prints bare: it starts with a digit
/// (so no leading sign) and is not integer-looking.
///
/// `ruleutils.c`'s `NUMERICOID` arm, verbatim — `isdigit(extval[0]) &&
/// strcspn(extval, "eE.") != strlen(extval)`. It is why `DEFAULT 1e3` on a numeric
/// column reads `'1000'::numeric` on 18.6 (the output text is `1000`, integer-looking)
/// while `DEFAULT 1.5` reads `1.5`.
pub fn numeric_prints_bare(text: &str) -> bool {
    text.as_bytes().first().is_some_and(u8::is_ascii_digit) && text.contains(['e', 'E', '.'])
}

/// A signed integer in `get_const_expr`'s `INT4OID` spelling, widened to `bigint` when
/// it does not fit `int4`.
///
/// Two rules in one: a negative `int4` is `'-1'::integer` so the text re-parses as a
/// constant rather than as `-` applied to one, and a value outside `int4` was never an
/// `int4` `Const` at all — `2147483648` is an `int8` literal, which takes the
/// `default:` arm and is therefore quoted and labelled whatever its sign (measured:
/// `'2147483648'::bigint`).
pub fn render_int_const(n: i64) -> String {
    if i32::try_from(n).is_ok() {
        if n < 0 {
            format!("'{n}'::integer")
        } else {
            n.to_string()
        }
    } else {
        format!("'{n}'::bigint")
    }
}

/// `get_const_expr`'s `NUMERICOID` arm over an already-rendered output text: bare when
/// [`numeric_prints_bare`] says the text looks like a float constant, quoted and
/// labelled otherwise.
///
/// The one home for the rule. `information_schema`'s `column_default` renderer had a
/// second copy of it, and its neighbour copy of [`render_int_const`] shipped beside a
/// string arm that did not fold at all — the class this function exists to prevent.
#[must_use]
pub fn render_numeric_const(text: &str) -> String {
    if numeric_prints_bare(text) {
        text.to_string()
    } else {
        format!("'{text}'::numeric")
    }
}

/// Render a literal, in the mode's own spelling and — in
/// [`RenderMode::Catalog`] — as the `Const` of type `ty` that 18.6 would have stored
/// there.
///
/// `ty` is the type an *unknown* literal resolved to at this position, `None` for a
/// position with no resolution (which is Postgres's `unknown` and renders unlabelled).
/// The arms below are `get_const_expr(showtype = 0)`: `bool` and `unknown` are never
/// labelled, a non-negative `int4` and a float-looking `numeric` print bare, and
/// everything else is a quoted literal with a `::type` label.
///
/// The three value-dependent folds — a string literal cast to `boolean`, `int4` or
/// `numeric` — reproduce the type's **output** function, which is what 18.6 prints:
/// `'007'::int4` reads `7` there and `'t'::bool` reads `true`. A spelling that does not
/// parse falls back to the parenthesized cast rather than inventing a canonical form;
/// such a value would be `22P02` at DDL on both engines anyway.
fn render_const(lit: &Literal, ty: Option<TypeOid>, mode: RenderMode<'_>) -> String {
    let RenderMode::Catalog(_) = mode else {
        return render_literal(lit);
    };
    match lit {
        // A NULL constant is always labelled when its type is known — 18.6 labels it
        // "to prevent misdecisions about type when reparsing" (measured:
        // `CHECK (a <> NULL)` reads `(a <> NULL::integer)`).
        Literal::Null => ty.map_or_else(
            || "NULL".to_string(),
            |t| catalog_type_name(t).map_or_else(|_| "NULL".to_string(), |n| format!("NULL::{n}")),
        ),
        Literal::Bool(b) => if *b { "true" } else { "false" }.to_string(),
        Literal::Int(n) => render_int_const(*n),
        Literal::Numeric(n) => render_numeric_const(&n.to_display_string()),
        Literal::Float(f) => f.to_string(),
        Literal::String(s) => render_catalog_string_const(s, ty, None),
    }
}

/// [`render_const`]'s string arm, and the **one** implementation of it: the *unknown*
/// literal `s`, folded into a `Const` of type `ty` and printed the way that type's
/// `get_const_expr` arm prints it.
///
/// `label` overrides the `::type` suffix spelling for the types that carry one — the
/// enum column case, where the value is a `Value::Text` and the type name lives on the
/// column rather than in [`TypeOid`].
///
/// **Every position that renders a string constant reaches this function**, the
/// `information_schema.columns.column_default` literal path included: a second copy
/// there labelled without folding, so `x bool DEFAULT 't'` read `'t'::boolean` against
/// 18.6's `true` and `y int DEFAULT '007'` read `'007'::integer` against `7` (both
/// measured). The type is the one the *position* resolved the unknown to, which for a
/// `DEFAULT` is `coerce_to_target_type`'s target — the **column's** own type, so an
/// `int8` column's `DEFAULT '007'` is `'7'::bigint` and not `7`: only `int4` takes
/// `get_const_expr`'s bare arm.
///
/// The value-dependent folds reproduce the type's **output** function. `bool`, `int4`
/// and `numeric` are the ones where the fold decides whether a label is printed at all;
/// `int2` and `int8` fold too, their output function being the same total decimal
/// rendering. Every other type prints the literal verbatim — `EDGES.md` row 93 — a
/// spelling that does not parse falling back to the parenthesized cast rather than
/// inventing a canonical form (such a value is `22P02` at DDL on both engines anyway).
#[must_use]
pub fn render_catalog_string_const(s: &str, ty: Option<TypeOid>, label: Option<&str>) -> String {
    let quoted = quote_catalog_string(s);
    let Some(ty) = ty else {
        // Postgres's `UNKNOWNOID` arm: quoted, never labelled.
        return quoted;
    };
    // An enum's own type name, which no `TypeOid` spells (a stored enum value is a
    // `Value::Text` with the name beside it on the column). Nothing folds under it —
    // an enum's output function is the identity on an accepted label.
    if let Some(name) = label {
        return format!("{quoted}::{name}");
    }
    match ty {
        TypeOid::Bool => match s.trim().to_ascii_lowercase().as_str() {
            "t" | "true" | "y" | "yes" | "on" | "1" => "true".to_string(),
            "f" | "false" | "n" | "no" | "off" | "0" => "false".to_string(),
            _ => format!("({quoted})::boolean"),
        },
        TypeOid::Int4 => s.trim().parse::<i32>().map_or_else(
            |_| format!("({quoted})::integer"),
            |n| render_int_const(i64::from(n)),
        ),
        // `int2` / `int8` are the same output function and a **different**
        // `get_const_expr` arm: the `default:` one, which quotes and labels whatever the
        // sign (`'7'::bigint`, `'7'::smallint` — measured, both from a `DEFAULT '007'`).
        TypeOid::Int2 => s.trim().parse::<i16>().map_or_else(
            |_| format!("({quoted})::smallint"),
            |n| format!("'{n}'::smallint"),
        ),
        TypeOid::Int8 => s.trim().parse::<i64>().map_or_else(
            |_| format!("({quoted})::bigint"),
            |n| format!("'{n}'::bigint"),
        ),
        TypeOid::Numeric => crate::types::PgNumeric::parse(s.trim()).map_or_else(
            |_| format!("({quoted})::numeric"),
            |n| render_numeric_const(&n.to_display_string()),
        ),
        other => catalog_type_name(other)
            .map_or_else(|_| quoted.clone(), |name| format!("{quoted}::{name}")),
    }
}

/// A SQL string literal in `ruleutils.c`'s `simple_quote_literal` spelling: `'…'` with
/// `''` doubling and **no** `E'…'` escape form.
///
/// [`quote_string_literal`]'s `E'…'` is a *dump* rule — it makes a backslash mean the
/// same byte under either `standard_conforming_strings`. Nothing re-parses a catalog
/// view, and 18.6 prints `'\x00'::bytea` there, so this is the spelling that agrees.
fn quote_catalog_string(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// The type spelling a cast prints, with a modifier written **inside** an array's
/// `[]` — one derivation for both [`RenderMode`]s.
///
/// A modifier is the **element's**: `varchar(3)[]` is an array of `varchar(3)`, which
/// is why `resolve_column_type` captures it at the element's width and why
/// `dump::column_type_sql` spells a column that way. Appending it after the whole name
/// emits `varchar[](3)` in [`RenderMode::Portable`] and `character varying[](3)` in
/// [`RenderMode::Catalog`] — one wrong spelling per mode, neither re-parsing on either
/// engine (`42601` on each), so a dump carrying one never reloads and a catalog reader
/// prints a value 18.6 does not.
///
/// Within the element name there are **two** positions, not one, and which applies is
/// [`catalog_type_name_with_modifier`]'s to say: `character varying(3)` and
/// `numeric(4,1)` take the modifier after the whole name, while the three multi-word
/// datetime spellings take it after the *first word*
/// (`timestamp(3) without time zone`, measured). [`RenderMode::Portable`] never faces
/// that split — every modifier-carrying type's portable name is one word — and it
/// must not, since its names have to re-parse (`timestamp(3)`).
fn cast_type_spelling(
    target: TypeOid,
    modspell: Option<&str>,
    mode: RenderMode<'_>,
) -> Result<String> {
    let Some(modspell) = modspell else {
        return match mode {
            RenderMode::Catalog(_) => catalog_type_name(target),
            RenderMode::Portable => type_oid_sql(target),
        };
    };
    let spelled = |oid| match mode {
        RenderMode::Catalog(_) => catalog_type_name_with_modifier(oid, modspell),
        RenderMode::Portable => Ok(format!("{}{modspell}", type_oid_sql(oid)?)),
    };
    match target.array_element() {
        Some(element) => Ok(format!("{}[]", spelled(element)?)),
        None => spelled(target),
    }
}

/// A [`RenderMode::Catalog`] scalar type name with its modifier at **18.6's own
/// position**, which for the three multi-word datetime names is *inside* the words.
///
/// `format_type` writes `timestamp(3) without time zone`, `timestamp(3) with time
/// zone` and `time(3) without time zone` — the modifier after the leading word, not
/// after the phrase — while `character varying(3)`, `numeric(4,1)` and `interval(3)`
/// take it after the whole name. Each measured on 18.6 through
/// `pg_get_constraintdef` and `format_type`, including the array form
/// (`timestamp(3) without time zone[]`: modifier inside, `[]` outside).
///
/// The `_` arm appends, so a modifier-carrying type whose catalog name is multi-word
/// would join the wrong half by default. That is why
/// `every_modifier_carrying_type_has_a_decided_modifier_position` fails on a
/// modifier-carrying type with no row of its own.
fn catalog_type_name_with_modifier(oid: TypeOid, modspell: &str) -> Result<String> {
    Ok(match oid {
        TypeOid::Timestamp => format!("timestamp{modspell} without time zone"),
        TypeOid::Timestamptz => format!("timestamp{modspell} with time zone"),
        TypeOid::Time => format!("time{modspell} without time zone"),
        other => format!("{}{modspell}", catalog_type_name(other)?),
    })
}

/// 18.6's `format_type_with_typemod` — a type named **with** the width it declares.
///
/// **What it serves, and what it does not.** Its one caller is the recursive-CTE
/// `42804`'s *type*-mismatch sentence (`parser::select`'s `check_recursive_shape`), where
/// 18.6 prints `format_type_with_typemod` at **both** halves and pglet used to print a
/// bare `type_oid_to_sql_name`: `has type numeric(6,2) in non-recursive term but type
/// double precision overall` here now, byte-identical to 18.6's, against the old
/// `numeric`. That is the whole of it. It is **not** required by the modifier fold beside
/// it — that fold raises nothing, so no message of its own exists to render — and a
/// reader should not infer a dependency between the two. The renderer was written for a
/// *refusal* on the modifier that was implemented, measured against the oracle, and
/// reversed (`docs/declared_types.md` § A recursive CTE declares a modifier it can
/// honour); it was kept because the surviving sentence is measurably closer to 18.6 with
/// it. Guarded by `a_type_and_its_atttypmod_spell_as_18_6_spells_them` at the renderer
/// and by `a_recursive_ctes_type_mismatch_names_the_width` at the sentence.
///
/// It is the pair `(oid, atttypmod)` a `RowDescription` field carries, spelled as
/// `format_type` spells it: the catalog name from [`catalog_type_name`], the modifier
/// decoded by [`pg_type_modifier_spelling`], and the position decided by
/// [`catalog_type_name_with_modifier`] — so `character varying(5)`, `numeric(6,2)` and
/// `timestamp(3) without time zone` each come out where 18.6 puts the `(…)`, and an
/// **array** takes its element's modifier *inside* the `[]` (`character varying(3)[]`,
/// measured on 18.6 in a recursive-CTE `42804`).
///
/// A `typmod` of `-1`, and a type carrying no modifier, spell the bare catalog name —
/// which is exactly what 18.6 prints for the "overall" side of a recursive query whose
/// two terms disagree, `select_common_typmod` having folded their modifiers away.
///
/// # Errors
///
/// [`catalog_type_name`]'s, which is unreachable — it is total over `TypeOid`.
pub fn format_type_with_typemod(oid: TypeOid, typmod: i32) -> Result<String> {
    let Some(modspell) = pg_type_modifier_spelling(oid, typmod) else {
        return catalog_type_name(oid);
    };
    match oid.array_element() {
        Some(element) => Ok(format!(
            "{}[]",
            catalog_type_name_with_modifier(element, &modspell)?
        )),
        None => catalog_type_name_with_modifier(oid, &modspell),
    }
}

/// Render a `::`-cast the way 18.6's `get_coercion_expr` does — including **not
/// rendering it at all**.
///
/// Three shapes, and the first is not a rendering rule but a parse-analysis fact:
///
/// - **The cast is not there.** `coerce_type` returns its argument untouched when the
///   argument's type already *is* the target, so 18.6 stores no node and
///   `pg_get_expr` prints the operand alone: `DEFAULT gen_random_uuid()::uuid` reads
///   `gen_random_uuid()`, `CHECK (s::text = 'x')` over a `text` column reads
///   `(s = 'x'::text)`. A typmod is the exception — a length coercion is a real node
///   over a same-type operand — so this arm is skipped when one is present.
/// - **The operand is a constant of the target type.** `get_coercion_expr` prints it
///   with `showtype = -1` and no parens, then appends the label: `'5'::text`,
///   `1.5::numeric(10,2)`. In pglet's tree the operand is an *unknown* literal that
///   this cast is what types, so every literal operand takes this arm.
/// - **Anything else** takes the parenthesized form pglet has always emitted, which is
///   already 18.6's: `(5)::bigint`, `((a)::text)`.
///
/// The type spelling itself — including where an array's modifier goes — is
/// [`cast_type_spelling`]'s, so the four positions below cannot disagree about it.
///
/// **One elision 18.6 makes is not made here**, and it is not this function's array
/// arm: `coerce_type` drops a cast whose target type *and typmod* both already match
/// the operand's, where the `modspell.is_none()` test below keeps every typmod'd cast.
/// So `CREATE INDEX ON sc1 ((s::varchar(3)))` over an `s varchar(3)` column reads
/// `btree (((s)::character varying(3)))` in `pg_indexes.indexdef` here against 18.6's
/// `btree (s)` — measured, at the scalar spelling as much as the array one, and open.
///
/// **The four temporal types joined it when their casts started carrying a typmod.**
/// Over an `a timestamp(3)` column, measured on both engines at `pg_get_constraintdef`,
/// `information_schema.check_constraints.check_clause` and `pg_indexes.indexdef`'s key
/// and predicate: `a::timestamp(3)` reads `((a)::timestamp(3) without time zone = a)`
/// here against 18.6's elided `(a = a)`, and `CREATE INDEX ix1 ON d1 ((a::timestamp(3)))`
/// reads `btree (((a)::timestamp(3) without time zone))` here against `btree (a)` there.
/// The **same** change moved four spellings the other way — `a::timestamp(6)`,
/// `b::timestamptz(1)`, `c::time(1)` and `d::interval(1)` over columns of a *different*
/// `p` are nodes 18.6 keeps, and each now prints byte-identically where all four were
/// elided before. The test here reads the type alone, so it disagrees with 18.6 at
/// exactly the spellings where the two typmods differ from what a type-only test
/// assumes; `a::timestamp` over an `a timestamp(3)` column is the mirror, elided here
/// and kept there (measured, and pre-existing). Closing it needs the **operand's**
/// typmod, which [`LeafScope`] does not carry.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
fn render_cast(
    expr: &Expr,
    target: TypeOid,
    modspell: Option<&str>,
    mode: RenderMode<'_>,
) -> Result<String> {
    // The operand is rendered from this frame — [`render_expr_mode`]'s, this one being
    // inlined into it — and the rest decided and spelled in frames of their own, so a
    // cast chain costs one frame a level (`crate::depth`).
    match cast_rendering(expr, target, modspell, mode)? {
        CastRendering::Done(rendered) => Ok(rendered),
        CastRendering::Operand => render_expr_mode(expr, mode),
        CastRendering::Coercion(spelling) => {
            let operand = render_expr_mode(expr, mode)?;
            Ok(coercion_spelling(&operand, &spelling))
        }
    }
}

/// How [`render_cast`] renders a cast: whole already, as its operand alone, or as a
/// coercion node around its rendered operand, spelled.
enum CastRendering {
    Done(String),
    Operand,
    Coercion(String),
}

#[inline(never)]
fn cast_rendering(
    expr: &Expr,
    target: TypeOid,
    modspell: Option<&str>,
    mode: RenderMode<'_>,
) -> Result<CastRendering> {
    let spelling = cast_type_spelling(target, modspell, mode)?;
    let RenderMode::Catalog(scope) = mode else {
        return Ok(CastRendering::Coercion(spelling));
    };
    let same_type = operand_type(expr, scope) == Some(target);
    // A cast over an operand that already has the target type is a node 18.6 never
    // built — unless a typmod makes it a length coercion, which is a real one.
    if modspell.is_none() && same_type {
        return Ok(CastRendering::Operand);
    }
    Ok(match expr {
        // An **unknown** literal is what this cast types, so 18.6's `Const` here is of
        // the target type and `get_coercion_expr` prints it undecorated: `'5'::text`,
        // never `('5')::text`. A typmod keeps the value bare and takes the label once.
        Expr::Literal(lit @ (Literal::String(_) | Literal::Null)) => {
            CastRendering::Done(if modspell.is_some() {
                format!("{}::{spelling}", render_const(lit, None, mode))
            } else {
                render_const(lit, Some(target), mode)
            })
        }
        // A *typed* constant of the target type under a length coercion — the
        // `consttypmod == -1` branch, again unparenthesized: `1.5::numeric(10,2)`.
        Expr::Literal(lit) if same_type => {
            CastRendering::Done(format!("{}::{spelling}", render_const(lit, None, mode)))
        }
        // Everything else is a genuine coercion node over an operand of another type,
        // which `get_coercion_expr` parenthesizes: `(5)::bigint`, `((a)::text)`.
        _ => CastRendering::Coercion(spelling),
    })
}

#[inline(never)]
fn coercion_spelling(operand: &str, spelling: &str) -> String {
    format!("({operand})::{spelling}")
}

/// Render an `IN` list as the node `transformAExprIn` builds for it.
///
/// pglet keeps the written `IN`; 18.6 has no such node at all, so the constraint the
/// spine's own `CHECK (level IN ('viewer', 'editor'))` reads back is a
/// `ScalarArrayOpExpr` there and an `IN` here. The transformation is reproduced at
/// **render** time and only in [`RenderMode::Catalog`]: normalising the stored plan
/// instead would change what every `InList` reader in the evaluator and the index
/// probe sees, and would make the dump emit `= ANY (ARRAY[…])`, which re-parses to
/// `Expr::AnyAll` — a different variant, breaking the round-trip invariant that keeps
/// a restored partial index serving the same queries.
///
/// `transformAExprIn`'s rules, from `parse_expr.c`:
///
/// - Elements are split by `contain_vars_of_level`: one holding a column stays an
///   operand of the boolean chain, the rest are candidates for the array.
/// - The array is built only when **more than one** non-column element remains, so
///   `a IN (1)` is `(a = 1)` and `a IN (1, b)` is `((a = 1) OR (a = b))`.
/// - `NOT IN` is the same transformation under `<>`, which flips the quantifier to
///   `ALL` and the chain to `AND`.
/// - The array comes **first** in the chain, then the column elements in list order
///   (measured: `a IN (b, c, 1, 2)` reads
///   `(((a = ANY (ARRAY[1, 2])) OR (a = b)) OR (a = c))`).
///
/// One rule is **declined rather than approximated**: `select_common_type` may pick a
/// type none of the operands has, and 18.6 then wraps every one of them in a coercion
/// (`a IN (1, 2.5)` reads `((a)::numeric = ANY (ARRAY[(1)::numeric, 2.5]))`). pglet
/// stores no such coercions, so when the elements do not already share the left
/// operand's type this falls back to the written `IN` — today's divergence, recorded
/// in `EDGES.md`, rather than a rendering that drops the casts and reads as a
/// different predicate.
fn render_in_list(
    expr: &Expr,
    rendered: Option<&str>,
    list: &[Expr],
    negated: bool,
    mode: RenderMode<'_>,
) -> Result<String> {
    // The left operand at a position resolving an unknown to `ty`: `rendered` when the
    // caller rendered it already, which it does for every shape but the ones a
    // position's type changes (`takes_position_type`).
    let left = |ty: Option<TypeOid>| {
        rendered.map_or_else(|| render_operand(expr, ty, mode), |r| Ok(r.to_owned()))
    };
    let RenderMode::Catalog(scope) = mode else {
        let items: Result<Vec<String>> = list.iter().map(|e| render_expr_mode(e, mode)).collect();
        let kw = if negated { "NOT IN" } else { "IN" };
        return Ok(format!("({} {kw} ({}))", left(None)?, items?.join(", ")));
    };
    let op = if negated { BinaryOp::Ne } else { BinaryOp::Eq };
    let sym = if negated { "<>" } else { "=" };
    let joiner = if negated { "AND" } else { "OR" };
    let (vars, nonvars): (Vec<&Expr>, Vec<&Expr>) = list.iter().partition(|e| contains_column(e));

    // `select_common_type` over the left operand and the array candidates, the left
    // operand first so it wins a tie — `common_element_type` declines (`None`) when two
    // known members disagree, which is exactly the case whose coercions this renderer
    // does not model, and answers `text` when every one of them is unknown.
    let elem_type =
        common_element_type(std::iter::once(expr).chain(nonvars.iter().copied()), scope);

    let mut parts: Vec<String> = Vec::new();
    let mut chain_elems: Vec<&Expr> = Vec::new();
    if nonvars.len() > 1 {
        let Some(elem_type) = elem_type else {
            // `select_common_type` would have coerced every operand to a type none of
            // them has, and pglet stores no such coercions. Emit the written `IN`
            // rather than a shape neither engine holds.
            let items: Result<Vec<String>> =
                list.iter().map(|e| render_expr_mode(e, mode)).collect();
            let kw = if negated { "NOT IN" } else { "IN" };
            return Ok(format!("({} {kw} ({}))", left(None)?, items?.join(", ")));
        };
        let members: Result<Vec<String>> = nonvars
            .iter()
            .map(|e| render_operand(e, Some(elem_type), mode))
            .collect();
        parts.push(format!(
            "({} {sym} {} (ARRAY[{}]))",
            left(Some(elem_type))?,
            if negated { "ALL" } else { "ANY" },
            members?.join(", ")
        ));
        chain_elems.extend(vars);
    } else {
        chain_elems.extend(list.iter());
    }
    for elem in chain_elems {
        // `render_binary_op`'s `=` / `<>`, whose operands take the type an unknown in
        // each position resolves to — the left one spliced in when already rendered.
        let (want_l, want_r) = binary_operand_wants(expr, op, elem, mode);
        let l = left(want_l)?;
        let r = render_operand(elem, want_r, mode)?;
        parts.push(render_binary_tail(op, &l, &r, mode)?);
    }
    // `transformAExprIn` **left-folds pairwise** — `makeBoolExpr(…, list_make2(result,
    // cmp))` inside the `foreach`, so an n-member chain is n-1 nested binary
    // `BoolExpr`s and `ruleutils.c` prints a paren per fold:
    // `a IN (1, 2, b, d)` is `(((a = ANY (ARRAY[1, 2])) OR (a = b)) OR (a = d))`, not the
    // flat n-ary join a *written* `a = 1 OR a = 2 OR a = 3` produces (that one is
    // `makeOrExpr`'s grammar-time flatten, and is [`render_bool_chain`]'s rule, not this
    // one — the two shapes were measured side by side on 18.6). The single comparison is
    // returned unwrapped: `a IN (1)` is `(a = 1)`, not `((a = 1))`.
    let mut iter = parts.into_iter();
    // An empty `IN ()` list is a parse error, so the first element is always there;
    // declining rather than indexing keeps this library code panic-free.
    let Some(first) = iter.next() else {
        return unrenderable("an empty IN list");
    };
    let folded = iter.fold(first, |acc, part| format!("({acc} {joiner} {part})"));
    Ok(folded)
}

/// Render one operand of a node that resolves an *unknown* literal at that position,
/// passing the resolved type down to the literal and leaving every other shape alone.
#[allow(clippy::inline_always)] // one frame a level, or no frame at a checkpoint (`crate::depth`)
#[inline(always)]
fn render_operand(expr: &Expr, ty: Option<TypeOid>, mode: RenderMode<'_>) -> Result<String> {
    match (expr, mode) {
        (Expr::Literal(lit), RenderMode::Catalog(_)) => Ok(render_const(lit, ty, mode)),
        (Expr::Array(items), RenderMode::Catalog(_)) => render_array(items, ty, mode),
        (Expr::Coalesce(args), RenderMode::Catalog(_)) => render_coalesce(args, ty, mode),
        _ => render_expr_mode(expr, mode),
    }
}

/// `ARRAY[…]`, whose elements take the array's element type: the operand type when the
/// position has one (`ar @> ARRAY['x']` over a `text[]` column reads
/// `ARRAY['x'::text]`), and `select_common_type` over the elements otherwise
/// (`DEFAULT ARRAY['a','b']` reads `ARRAY['a'::text, 'b'::text]`).
fn render_array(items: &[Expr], ty: Option<TypeOid>, mode: RenderMode<'_>) -> Result<String> {
    let elem = match mode {
        RenderMode::Portable => None,
        RenderMode::Catalog(scope) => ty
            .and_then(TypeOid::array_element)
            .or_else(|| common_element_type(items, scope)),
    };
    let rendered: Result<Vec<String>> = items
        .iter()
        .map(|e| render_operand(e, elem, mode))
        .collect();
    Ok(format!("ARRAY[{}]", rendered?.join(", ")))
}

/// `COALESCE(…)`, whose arguments are all coerced to one common type — so an *unknown*
/// among them is labelled with it: `COALESCE(s, 'd')` over a `text` column reads
/// `COALESCE(s, 'd'::text)`.
fn render_coalesce(args: &[Expr], ty: Option<TypeOid>, mode: RenderMode<'_>) -> Result<String> {
    let common = match mode {
        RenderMode::Portable => None,
        RenderMode::Catalog(scope) => common_element_type(args, scope).or(ty),
    };
    let rendered: Result<Vec<String>> = args
        .iter()
        .map(|e| render_operand(e, common, mode))
        .collect();
    Ok(format!("COALESCE({})", rendered?.join(", ")))
}

/// `select_common_type` for a list all of whose known members already agree: the first
/// type any member carries, `text` when every one of them is *unknown*, and `None` when
/// two disagree — the case whose coercions this renderer does not model.
///
/// `Varchar` is **not** folded to `Text` here, unlike in [`unknown_resolves_to`], and the
/// two placements are each where their own measurement puts them: over a `varchar(10)`
/// column 18.6 renders `v = 'x'` as `((v)::text = 'x'::text)` — the literal is text —
/// but renders `v IN ('x','y')` as
/// `((v)::text = ANY ((ARRAY['x'::character varying, 'y'::character varying])::text[]))`,
/// where `select_common_type` picked *varchar* (the left operand is first and so wins the
/// tie) and the whole array is cast afterwards. pglet renders neither outer cast
/// (`EDGES.md` row 90); what it can get right is which type the elements carry.
fn common_element_type<'a>(
    items: impl IntoIterator<Item = &'a Expr>,
    scope: &dyn LeafScope,
) -> Option<TypeOid> {
    let mut found: Option<TypeOid> = None;
    for item in items {
        if let Some(t) = operand_type(item, scope) {
            match found {
                None => found = Some(t),
                Some(prev) if prev == t => {}
                Some(_) => return None,
            }
        }
    }
    Some(found.unwrap_or(TypeOid::Text))
}

#[cfg(test)]
mod catalog_case_render_tests {
    use super::render_catalog_expr;
    use crate::catalog::Schema;
    use crate::planner::{Expr, Literal, PlannerScope};

    /// A `CASE`'s branch results are a `select_common_type` position, so an *unknown*
    /// literal there is stored as a typed `Const` on 18.6 and prints with its label:
    /// `CASE WHEN (a = 1) THEN 'y' ELSE 'n' END` reads `THEN 'y'::text … ELSE
    /// 'n'::text` (measured against `pg_get_constraintdef`).
    ///
    /// **No differential fixture can hold this**, which is why it is a unit test:
    /// `pg_get_expr` runs with `PRETTYFLAG_INDENT`, so 18.6 renders every `CASE` over
    /// four indented lines and pglet renders it on one (`EDGES.md` row 91) — a faithful
    /// fixture is red on the formatting whatever the constants do.
    #[test]
    fn case_branch_unknowns_take_the_common_type() {
        let schema = Schema::new();
        let scope = PlannerScope::none(&schema);
        let case = Expr::Case {
            operand: None,
            when_clauses: vec![(
                Expr::Literal(Literal::Bool(true)),
                Expr::Literal(Literal::String("y".into())),
            )],
            else_result: Some(Box::new(Expr::Literal(Literal::String("n".into())))),
        };
        assert_eq!(
            render_catalog_expr(&case, &scope).unwrap(),
            "CASE WHEN true THEN 'y'::text ELSE 'n'::text END"
        );
        // The `int` branches stay bare — the fold is the resolved type's rule, not a
        // blanket label.
        let ints = Expr::Case {
            operand: None,
            when_clauses: vec![(
                Expr::Literal(Literal::Bool(true)),
                Expr::Literal(Literal::Int(1)),
            )],
            else_result: Some(Box::new(Expr::Literal(Literal::Int(2)))),
        };
        assert_eq!(
            render_catalog_expr(&ints, &scope).unwrap(),
            "CASE WHEN true THEN 1 ELSE 2 END"
        );
        // An ELSE-less `CASE` still stores one on 18.6 (`transformCaseExpr` supplies a
        // NULL `defresult`), labelled with the result type.
        let no_else = Expr::Case {
            operand: None,
            when_clauses: vec![(
                Expr::Literal(Literal::Bool(true)),
                Expr::Literal(Literal::String("y".into())),
            )],
            else_result: None,
        };
        assert_eq!(
            render_catalog_expr(&no_else, &scope).unwrap(),
            "CASE WHEN true THEN 'y'::text ELSE NULL::text END"
        );
        // ...and `Portable` must not add it, or the round-trip lands on a different
        // variant.
        assert_eq!(
            super::render_expr(&no_else).unwrap(),
            "CASE WHEN TRUE THEN 'y' END"
        );
    }
}

#[cfg(test)]
mod catalog_type_name_tests {
    use super::catalog_type_name;
    use crate::types::TypeOid;

    /// Every `TypeOid` has a catalog spelling — the claim `catalog_type_name`'s doc
    /// makes, enumerated over the wire-OID space rather than over a hand list, so a
    /// variant added with a new OID is covered by the same walk.
    ///
    /// `name` (19) and `name[]` (1003) are the two the doc used to call unspellable,
    /// so they are asserted by value as well as by the sweep.
    #[test]
    fn catalog_type_name_is_total() {
        let mut seen = 0;
        for raw in 0..5000_i32 {
            if let Some(oid) = TypeOid::from_oid(raw) {
                seen += 1;
                assert!(
                    catalog_type_name(oid).is_ok(),
                    "no catalog spelling for {oid:?}"
                );
            }
        }
        assert!(seen >= 30, "the OID sweep found only {seen} types");
        assert_eq!(catalog_type_name(TypeOid::Name).unwrap(), "name");
        assert_eq!(catalog_type_name(TypeOid::NameArray).unwrap(), "name[]");
    }

    /// **A modifier goes where 18.6 puts it, and there are three positions.** Each
    /// string below was read off 18.6 through `pg_get_constraintdef` over a
    /// `CHECK (col::T(p) = col)` whose `p` differs from the column's (so
    /// `coerce_type` keeps the node), and the array one off
    /// `format_type(atttypid, atttypmod)`.
    ///
    /// The three multi-word datetime names take it **inside** the words; everything
    /// else takes it after the whole name. Rendering `character varying(3)`'s rule at
    /// a `timestamp` emits `timestamp without time zone(3)`, which is not what any
    /// 18.6 catalog reader prints.
    #[test]
    fn a_catalog_modifier_goes_where_18_6_puts_it() {
        let spell = |oid, m| super::catalog_type_name_with_modifier(oid, m).unwrap();
        assert_eq!(
            spell(TypeOid::Timestamp, "(3)"),
            "timestamp(3) without time zone"
        );
        assert_eq!(
            spell(TypeOid::Timestamptz, "(3)"),
            "timestamp(3) with time zone"
        );
        assert_eq!(spell(TypeOid::Time, "(3)"), "time(3) without time zone");
        assert_eq!(spell(TypeOid::Interval, "(3)"), "interval(3)");
        assert_eq!(spell(TypeOid::Varchar, "(3)"), "character varying(3)");
        assert_eq!(spell(TypeOid::Numeric, "(4,1)"), "numeric(4,1)");
        // The array form spells the **element** with its modifier and puts the `[]`
        // outside the words — 18.6's `format_type` of a `timestamp(3)[]` column.
        assert_eq!(
            super::cast_type_spelling(
                TypeOid::TimestampArray,
                Some("(3)"),
                super::RenderMode::Catalog(&crate::planner::PlannerScope::none(
                    &crate::catalog::Schema::new()
                ))
            )
            .unwrap(),
            "timestamp(3) without time zone[]"
        );
    }

    /// **Where a modifier goes is a per-type fact, and every type that carries one
    /// has to have decided.** The position is not predictable from the shape of the
    /// name: `character varying(3)` is multi-word and takes it *after* the phrase,
    /// while `timestamp(3) without time zone` takes it after the first word — both
    /// measured on 18.6 — so [`super::catalog_type_name_with_modifier`]'s `_` arm
    /// (which appends) is a decision, not a default.
    ///
    /// Nothing in the compiler forces that decision for a new modifier-carrying type,
    /// so this pins the set: the modifier-carrying element types the OID sweep finds
    /// must be exactly the ones tabulated here, each with the spelling 18.6 prints.
    /// A type joining `type_oid_carries_modifier`'s `true` half reddens this until
    /// someone writes its row.
    ///
    /// [`RenderMode::Portable`](super::RenderMode::Portable) has no such split — the
    /// modifier always follows the whole name — so its column is here to pin that
    /// every one of those names is a **single word that re-parses**, which is what a
    /// dump carrying `timestamp(3)` depends on.
    #[test]
    fn every_modifier_carrying_type_has_a_decided_modifier_position() {
        // (type, modifier, catalog spelling, portable spelling) — each catalog
        // spelling read off 18.6 (`pg_get_constraintdef` / `format_type`).
        let table = [
            (
                TypeOid::Varchar,
                "(3)",
                "character varying(3)",
                "varchar(3)",
            ),
            (TypeOid::Numeric, "(4,1)", "numeric(4,1)", "numeric(4,1)"),
            (
                TypeOid::Timestamp,
                "(3)",
                "timestamp(3) without time zone",
                "timestamp(3)",
            ),
            (
                TypeOid::Timestamptz,
                "(3)",
                "timestamp(3) with time zone",
                "timestamptz(3)",
            ),
            (TypeOid::Time, "(3)", "time(3) without time zone", "time(3)"),
            (TypeOid::Interval, "(3)", "interval(3)", "interval(3)"),
        ];
        for (oid, modifier, catalog, portable) in table {
            assert_eq!(
                super::catalog_type_name_with_modifier(oid, modifier).unwrap(),
                catalog,
                "{oid:?} catalog spelling"
            );
            assert_eq!(
                format!("{}{modifier}", super::type_oid_sql(oid).unwrap()),
                portable,
                "{oid:?} portable spelling"
            );
        }
        // The set, swept over the wire-OID space rather than listed by hand, so a new
        // variant is covered by the same walk. Arrays answer at their element's width,
        // which is why the sweep folds them onto the element.
        let mut carriers: Vec<TypeOid> = Vec::new();
        for raw in 0..5000_i32 {
            let Some(oid) = TypeOid::from_oid(raw) else {
                continue;
            };
            if !crate::types::type_oid_carries_modifier(oid) {
                continue;
            }
            let element = oid.array_element().unwrap_or(oid);
            if !carriers.contains(&element) {
                carriers.push(element);
            }
        }
        carriers.sort_by_key(|o| format!("{o:?}"));
        let mut tabulated: Vec<TypeOid> = table.iter().map(|r| r.0).collect();
        tabulated.sort_by_key(|o| format!("{o:?}"));
        assert_eq!(
            carriers, tabulated,
            "a modifier-carrying type has no decided modifier position"
        );
    }

    /// **18.6's `format_type_with_typemod`, from the `(oid, atttypmod)` pair a
    /// `RowDescription` field carries** — the encoded modifier decoded, spelled, and
    /// placed. Each string below was read off 18.6's own `42804` for a recursive CTE
    /// whose two terms disagree, which is the one message in this engine that prints a
    /// type twice and needs the width to tell the halves apart.
    ///
    /// The **array** row is the reason this is not just the position test above: an
    /// array's modifier is its element's and is spelled *inside* the `[]`
    /// (`character varying(3)[]`, measured), so a renderer appending after the whole
    /// name would print `character varying[](3)`.
    #[test]
    fn a_type_and_its_atttypmod_spell_as_18_6_spells_them() {
        for (oid, typmod, spelling) in [
            (TypeOid::Varchar, 9, "character varying(5)"),
            (TypeOid::Varchar, -1, "character varying"),
            (TypeOid::Numeric, 393_222, "numeric(6,2)"),
            (TypeOid::Numeric, -1, "numeric"),
            (TypeOid::Timestamp, 3, "timestamp(3) without time zone"),
            (TypeOid::Timestamp, -1, "timestamp without time zone"),
            (TypeOid::Interval, 2_147_418_115, "interval(3)"),
            (TypeOid::VarcharArray, 7, "character varying(3)[]"),
            (TypeOid::VarcharArray, -1, "character varying[]"),
            (TypeOid::Int4, -1, "integer"),
            // A type carrying no modifier ignores a value in the field rather than
            // inventing a `(…)` for it.
            (TypeOid::Int4, 9, "integer"),
        ] {
            assert_eq!(
                super::format_type_with_typemod(oid, typmod).unwrap(),
                spelling,
                "{oid:?} at atttypmod {typmod}"
            );
        }
    }
}

#[cfg(test)]
mod catalog_fn_render_tests {
    use super::render_expr;
    use crate::planner::{Expr, Literal, StringFunc};

    /// The two catalog-reading `StringFunc`s render to the form they were **written**
    /// as, not to a uniform `name(args)` call.
    ///
    /// They are the pair `pglet::dump`'s `ROUND_TRIP_CORPUS` cannot reach (a stored
    /// expression's DDL-time probe lends no catalog, so neither can be persisted), so
    /// the arms that render them have no corpus coverage and are proven here instead.
    /// `::regclass` is the load-bearing one: `SELECT regclass('t')` is
    /// `0A000 function not supported: regclass` here, so a renderer emitting
    /// `regclass(x)` would write SQL **pglet** does not reload. 18.6 answers that
    /// spelling, so the reason is pglet's alone.
    #[test]
    fn catalog_reading_string_fns_render_to_their_written_form() {
        let arg = Expr::Literal(Literal::Int(1));
        let regclass = Expr::StringFn {
            func: StringFunc::Regclass,
            args: vec![arg.clone()],
        };
        assert_eq!(render_expr(&regclass).unwrap(), "(1)::regclass");
        let constraintdef = Expr::StringFn {
            func: StringFunc::PgGetConstraintdef,
            args: vec![arg],
        };
        assert_eq!(
            render_expr(&constraintdef).unwrap(),
            "pg_get_constraintdef(1)"
        );
    }
}
