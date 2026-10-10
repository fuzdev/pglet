//! pglet's native, fused SQL parser: SQL text → planner `Expr`/`Plan` directly.
//!
//! Built as `text → lexer → recursive-descent + Pratt → Expr/Plan`. The parser is
//! **fully fused** — it emits the planner IR straight away (no intermediate AST,
//! no second traversal) and returns the crate [`Error`], so per-construct
//! SQLSTATE (`42601` syntax / `42704` unsupported type / `0A000` unsupported
//! construct) is raised at the point of parse. See `docs/parser.md`.
//!
//! The expression engine is a **Pratt-core hybrid**: a single
//! `parse_expr_bp(min_bp)` loop over a *pure* binary-operator binding-power table
//! (see [`expr`]), with the keyword-led forms that aren't clean binary operators
//! (`NOT`, the `IS …` postfix family, `BETWEEN`/`IN`/`LIKE`, `::`, …) as explicit
//! prefix/postfix productions. A depth guard caps nesting so a hostile
//! deeply-nested input can't overflow the stack while it is parsed. The trees the
//! parser *builds* — a long flat operator chain among them, which the guard does not
//! see — are bounded separately, where each is built ([`crate::depth`]).

mod ddl;
mod dml;
mod expr;
mod plpgsql;
mod select;
mod stream;

pub(crate) use stream::lex_error_to_error;
mod type_name;
mod with_recursive;

#[cfg(test)]
mod fuzz;
#[cfg(test)]
mod operator_loop_arms;

/// The four persisted-`Expr` contexts, so the executor's DDL-time walks phrase their
/// refusals in Postgres's own words. The three `CHECK`/index contexts are refused in
/// this module; a column `DEFAULT`'s are refused in `executor::ddl`, at the position
/// 18.6 cooks a default — which is why the enum is shared rather than local.
pub use ddl::PersistedExprContext;

use std::collections::HashMap;

use crate::catalog::Schema;
use crate::lexer::{Keyword, Lexer, PgClass, TokenKind};
use crate::planner::{
    Expr, Plan, SetOpKind, SetOpOuterTypes, SetOpTyping, SideReferenceTypes, build_set_op,
};
use crate::types::TypeOid;
use crate::{Error, Result};
use stream::TokenStream;

/// Maximum expression nesting depth before failing loud.
///
/// The recursive-descent + Pratt parser recurses on *nesting* (parens, chained
/// prefix ops, subqueries) — flat operator chains stay loop-iterative in the bp
/// loop. A hostile deeply-nested input would otherwise overflow the stack; this
/// cap (Postgres's `max_stack_depth` in spirit) bounds the recursion so it can't.
///
/// Set conservatively: a parenthesis level costs ~5 stack frames, so the cap must
/// fire well within the smallest reasonable thread stack (a 2 MB worker, not just
/// the 8 MB main thread) — and a debug build's frames are several times larger
/// than release's. 128 is far beyond any real query's nesting.
///
/// **The nesting this admits is also charged to the tree-depth limit, and each level
/// takes the byte check** ([`crate::depth`], in [`Parser::enter`]): a debug build's
/// frames are what a 2 MiB thread runs out of first — without the byte check, 97 nested
/// `abs(…)` calls, well inside this cap, overflowed one; with it, a debug embed answers
/// 78 on a declared 2 MiB thread and 63 on an undeclared one, and refuses `54001` past
/// them (measured). Natively the byte check is the backstop; on wasm, which has none, this
/// cap and the depth limit are, and this cap and `pglet_core`'s `MAX_ERROR_SIZE` are two
/// halves of one product — depth × frame size, the frame size moving with `Error`, since
/// every fallible frame holds `Result<_, Error>` slots. `crates/pglet/tests/stack_depth.rs`
/// measures the native answer; natively no test sees the product's cliff (see
/// `fuzz::deep_nesting_fails_loud_not_overflow`).
const MAX_DEPTH: usize = 128;

/// Whether `kw` is a **reserved** keyword — one that cannot serve as a bare
/// identifier (column / table / alias / constraint name).
///
/// **Derived, not hand-listed.** A word is reserved exactly when Postgres's
/// `ColId` grammar — the production for a bare column reference, a DDL column /
/// table name, and an object name — does **not** admit it. `ColId` is
/// `IDENT | unreserved_keyword | col_name_keyword`, so a keyword is a valid bare
/// name iff its [`PgClass`] is [`Unreserved`](PgClass::Unreserved) or
/// [`ColName`](PgClass::ColName); everything else
/// ([`Reserved`](PgClass::Reserved) and [`TypeFuncName`](PgClass::TypeFuncName))
/// is reserved here. This makes pglet's bare-name acceptance match Postgres's by
/// construction: `update`/`set`/`by`/`values`/`between` are accepted, while
/// `select`/`from` (`Reserved`) and `is`/`like`/`left`/`join` (`TypeFuncName`)
/// are rejected — and Postgres rejects those last four as bare columns too
/// (`SELECT is FROM t` is a syntax error in both).
///
/// The `TypeFuncName` words are usable as identifiers only in Postgres's broader
/// `ColLabel` positions — a qualified attribute (`t.left`) and an `AS` label
/// (`… AS left`) — which the parser handles separately (see
/// `parse_identifier_primary` and `expect_col_label`). A **bare** (no-`AS`)
/// keyword label stays unsupported: it would collide with clause markers
/// (`UPDATE t SET …`, `FROM a left JOIN b`), so pglet keeps it fail-loud,
/// matching Postgres's own bare-label restrictions closely enough for the spine.
const fn is_reserved(kw: Keyword) -> bool {
    // Reserved here == "not a valid Postgres `ColId`".
    !matches!(kw.pg_class(), PgClass::Unreserved | PgClass::ColName)
}

/// Whether `kind` may serve as a Postgres `ColId` name token: a bare or quoted
/// identifier, or a non-reserved keyword.
///
/// The single predicate behind every `ColId`-name position — column / table /
/// alias / index-method / constraint names — so one position can't silently drift
/// stricter than the others (the `parse_table_factor` Identifier-only regression
/// the FROM-table-name fix corrected was exactly this drift). The wider `ColLabel`
/// positions (`AS` labels, qualified attributes), which admit *any* keyword, are a
/// deliberately separate predicate (`expect_col_label`, `parse_identifier_primary`'s
/// dotted parts), not this one.
const fn is_colid_name(kind: &TokenKind) -> bool {
    match kind {
        TokenKind::Identifier | TokenKind::QuotedIdentifier => true,
        TokenKind::Keyword(k) => !is_reserved(*k),
        _ => false,
    }
}

/// The parts joined by `.` for an error message — Postgres's `NameListToString`,
/// which likewise renders the resolved text without re-quoting it
/// (`relation "PUBLIC.s" does not exist`).
fn join_name_parts(parts: &[String]) -> String {
    parts.join(".")
}

/// Reduce a (possibly schema-qualified) dotted object name to a single table name
/// under pglet's one implicit `public` schema. The model — strip `public.`, fail
/// loud on anything else — replaces the old accept-and-drop that silently truncated
/// any qualifier to its last part (so `noschema.users` resolved to `users`):
///
/// - a bare `name` is itself;
/// - `public.name` strips to `name` (the implicit schema). The test is **exact**, and
///   it can be, because every part reaching here is already in Postgres's canonical
///   form: an unquoted spelling was downcased where the source was read
///   ([`crate::lexer::Token::name`] for an identifier, [`split_name_literal`] for a
///   name literal) and a quoted one kept verbatim. So `PUBLIC.s` and `"public".s`
///   both strip, and `"PUBLIC".s` does not — matching 18.6 in all three;
/// - any **other** 2-part schema qualifier fails loud `42P01`. That matches Postgres on
///   a **lookup** (`SELECT * FROM noschema.qq` → `relation "noschema.qq" does not
///   exist`) and diverges where Postgres resolves the schema first and answers
///   `3F000 schema "noschema" does not exist` — measured on 18.6 with **unquoted**
///   names, so quoting is not the obstacle: `SELECT` / `INSERT` / `UPDATE` / `DELETE`
///   all answer `42P01` (pglet agrees, no divergence), while `CREATE TABLE`,
///   `DROP TABLE`, `CREATE INDEX … ON noschema.t`, `ALTER TABLE`, `CREATE SEQUENCE` and
///   `TRUNCATE` all answer `3F000`. `Error::SchemaNotFound` (`3F000`) is already
///   modelled, so the fix is reachable: it needs a statement context here. Note it is
///   *live for names nobody quotes*, so it is not a quoting question at all.
///   **Not every caller of this reduction is a relation lookup**, and the ones that are
///   not take a third answer — see `EDGES.md`, which carries
///   the record;
/// - **3 or more** parts fail loud `42601` (`improper qualified name`), refusing the
///   `db.schema.table` form pglet can't model. Postgres splits that into **three**
///   answers, and only the last of them is the too-many-dotted-names error this
///   arm was written against: at **4+** parts `makeRangeVarFromNameList` raises
///   `42601 improper relation name (too many dotted names)` — same SQLSTATE, near-same
///   wording; at exactly **3** it reaches `RangeVarGetRelidExtended`, which **resolves**
///   the name when the leading part is the current database and otherwise raises
///   `0A000 cross-database references are not implemented`. So this arm matches
///   Postgres at 4+ parts and diverges at 3, in both directions — measured on 18.6 with
///   `nextval('a.b.c')` (`0A000` there, `42601` here) and `nextval('<current db>.public.s')`
///   (a row there, `42601` here).
///
/// A catalog (virtual) table is matched on its full qualified name *before* this
/// reduction (`information_schema.columns`, `pg_catalog.pg_class`), so it never
/// reaches here.
///
/// **This is not the only place that decides what `public` means**, and the two others
/// — `executor::ddl`'s `execute_create_schema` and `execute_drop_schema`, which take a
/// statement's schema name rather than a qualifier — now decide it the same way, by an
/// exact test over an already-folded name. They could not before: while the engine
/// compared identifiers case-insensitively, `DROP SCHEMA "PUBLIC" CASCADE` matched and
/// dropped the real `public` schema with every table in it.
///
/// `pub(crate)` so the sequence-function name resolution (`build_call`) reduces a
/// `nextval`/`currval`/`setval` name through the **same** path as `CREATE SEQUENCE`
/// / `DEFAULT nextval(...)`, rather than a divergent copy that silently dropped a
/// non-`public` qualifier.
pub(crate) fn reduce_qualified_name(parts: &[String]) -> Result<String> {
    match parts {
        [name] => Ok(name.clone()),
        [schema, name] if schema == "public" => Ok(name.clone()),
        [_schema, _name] => Err(crate::CatalogError::TableNotFound {
            table: join_name_parts(parts),
        }
        .into()),
        _ => Err(Error::syntax(format!(
            "improper qualified name (too many dotted names): {}",
            join_name_parts(parts)
        ))),
    }
}

/// Read a relation name out of a **string literal** and reduce it to the single bare
/// name its caller resolves.
///
/// Two kinds of caller, and a grep for this function is their enumeration: the
/// sequence-name positions (a `nextval`/`currval`/`setval` argument, at plan time and in
/// a column `DEFAULT`), which key `Schema.sequences`; and `executor::expr`'s
/// `eval_regclass`, which keys `schema.tables` / `schema.indexes` and then renders the
/// name it got back, through [`crate::lexer::quote_ident`], as a **value a client reads**.
/// That second caller makes this reader's answer a value a client reads, so its
/// divergences surface there and not only at a restore: `'12345'::regclass` and
/// `'-'::regclass` are `42P01` here against 18.6's `12345` and `-`, and
/// `E'\x0Bqt'::regclass` over a live `qt` is `42P01` here against `qt` there (measured).
///
/// A name in a literal is not the same lexical object as a name in an identifier
/// position, and reading it as one (splitting on `.` and taking the parts verbatim)
/// is what let `nextval('"s"')` name a relation *whose name contains the quote
/// characters*. Postgres reads it with `SplitIdentifierString`, and this is the same
/// grammar: leading/trailing whitespace is skipped, a part beginning `"` runs to the
/// first `"` not doubled (with `""` folding to one `"`), an unquoted part runs to the
/// next `.` or whitespace, and anything else — an empty name, a second unquoted word
/// (`'s x'`), an unterminated `"` — is [`Error::InvalidName`] (`42602`), matching
/// Postgres exactly. An empty *quoted* part is legal syntax on both (`'""'` names the
/// relation `""`, which no catalog holds).
///
/// An **unquoted part is downcased** and a quoted one taken verbatim, the same fold
/// [`crate::lexer::Token::name`] applies in an identifier position — which is what
/// makes the two positions name the same relation: `CREATE SEQUENCE MySeq` registers
/// `myseq` and `nextval('MYSEQ')` finds it, while `nextval('"MySeq"')` does not.
/// Postgres does it in `SplitIdentifierString` itself, with
/// `downcase_truncate_identifier` over each unquoted token.
///
/// The `.`-separated parts then reduce through [`reduce_qualified_name`], so a
/// literal keys the same slot `CREATE SEQUENCE` does.
///
/// **Three divergences from `SplitIdentifierString`, none of them incidental:**
///
/// 1. **No `truncate_identifier` to `NAMEDATALEN`.** Postgres truncates *every* part,
///    quoted included, so a 64-byte-plus name resolves there and does not here.
///    Self-consistent within pglet — its lexer does not truncate an identifier either
///    — so the two name positions still agree and no dump round-trips wrong.
/// 2. **`is_scanner_space` omits `\v`** (0x0b), which `scanner_isspace` accepts:
///    `nextval(E'\x0Bs')` resolves `s` on 18.6 and is `42P01` here, while the form
///    feed `E'\x0Cs'` beside it resolves on both.
/// 3. **An all-digit literal — or a lone `-` — is a name here and an OID there.**
///    `regclassin` runs `parseDashOrOid` *before* `stringToQualifiedNameList`, and that
///    function is named for its two inputs (the PostgreSQL source's own comment: `/* Handle "-"
///    or numeric OID */`), so both spellings leave the name grammar entirely:
///    `nextval('12345')` is `XX000 could not open relation with OID 12345` on 18.6 and
///    `nextval('-')` is `XX000 could not open relation with OID 0`, against `42P01` here
///    for each. The dump cannot reach either: `quote_name_literal` quotes any name that
///    is not a safe bare identifier, and neither a digit string nor `-` is one, so the
///    emitted `'"12345"'` / `'"-"'` is a name on both engines.
pub(crate) fn reduce_name_literal(literal: &str) -> Result<String> {
    reduce_qualified_name(&split_name_literal(literal)?)
}

/// Whether `c` is whitespace to Postgres's `scanner_isspace` — the ASCII set that
/// function accepts, **minus `\v`**, and deliberately not `char::is_whitespace`, whose
/// Unicode spaces (NBSP and friends) are ordinary name characters to Postgres.
///
/// The `\v` gap is a real divergence, not a rounding of the set: `scanner_isspace`
/// accepts `\v`, so `nextval(E'\x0Bs')` resolves `s` on 18.6 where this reader answers
/// `42P01` (the neighbouring `\x0C` form feed resolves on both). It is recorded rather
/// than closed, and **not** because the same set governs pglet's lexer: it does not —
/// [`crate::lexer::is_space`], the lexer's set, accepts `0x0b`, so the two sets disagree
/// and closing this one would make them agree. Adding `\v` here is a one-character
/// change with no coupling to anything; it wants its own measurement and fixture.
const fn is_scanner_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\n' | '\r' | '\x0c')
}

/// Split a name literal into its dot-separated parts, per Postgres's
/// `SplitIdentifierString`: an unquoted part is downcased, a quoted one taken
/// verbatim. See [`reduce_name_literal`] for the grammar and the three deliberate
/// divergences.
// The quoted and unquoted arms both open `let mut name = String::new()`, which
// `branches_sharing_code` wants hoisted. They stay apart deliberately: this reader is
// the name-literal half of the one fold (`Token::name` is the other), and the two arms
// are the *quoted-verbatim* and *unquoted-downcased* rules. Sharing a binding across
// them makes the split read as incidental.
#[allow(clippy::branches_sharing_code)]
pub(crate) fn split_name_literal(literal: &str) -> Result<Vec<String>> {
    let mut chars = literal.chars().peekable();
    let mut parts = Vec::new();
    while chars.peek().is_some_and(|c| is_scanner_space(*c)) {
        chars.next();
    }
    if chars.peek().is_none() {
        // An empty (or all-whitespace) literal yields no names at all, which
        // `stringToQualifiedNameList` rejects rather than resolving to anything.
        return Err(Error::InvalidName);
    }
    loop {
        if chars.next_if_eq(&'"').is_some() {
            let mut name = String::new();
            loop {
                let Some(c) = chars.next() else {
                    return Err(Error::InvalidName); // mismatched quotes
                };
                if c != '"' {
                    name.push(c);
                } else if chars.next_if_eq(&'"').is_some() {
                    name.push('"'); // a doubled `""` is one literal quote
                } else {
                    break;
                }
            }
            parts.push(name); // quoted — exact
        } else {
            let mut name = String::new();
            while let Some(&c) = chars.peek() {
                if c == '.' || is_scanner_space(c) {
                    break;
                }
                name.push(c);
                chars.next();
            }
            if name.is_empty() {
                return Err(Error::InvalidName); // empty token
            }
            // Unquoted — `downcase_truncate_identifier`, minus the truncation.
            parts.push(name.to_ascii_lowercase());
        }
        while chars.peek().is_some_and(|c| is_scanner_space(*c)) {
            chars.next();
        }
        match chars.next() {
            None => return Ok(parts),
            // Whatever follows a name that is neither a separator nor the end — a
            // second unquoted word, a `"` opening mid-name — is invalid rather than
            // silently truncating to the part already read.
            Some(c) if c != '.' => return Err(Error::InvalidName),
            Some(_) => {
                while chars.peek().is_some_and(|c| is_scanner_space(*c)) {
                    chars.next();
                }
            }
        }
    }
}

/// The recursive-descent + Pratt parser over a token stream.
#[allow(clippy::struct_excessive_bools)] // independent facts the parse records, one per flag
struct Parser<'a> {
    ts: TokenStream<'a>,
    /// Schema context — consulted for FROM-table resolution, WHERE
    /// type-resolution, subquery planning, and `CAST`-to-user-`ENUM`.
    schema: &'a Schema,
    /// Current nesting depth, bounded by [`MAX_DEPTH`].
    depth: usize,
    /// `WITH` common-table-expression subplans in scope for the current statement,
    /// keyed by CTE name. Populated by `parse_with_clause` and consulted when a FROM
    /// table factor names a CTE; cleared after the statement (CTEs are statement-scoped).
    ///
    /// The value is a [`CteEntry`], not a bare `Plan`, because a CTE's `name(cols)`
    /// list renames the relation's columns **on every reader** and the body plan alone
    /// cannot carry that: an ordinary CTE's two reference kinds both re-derive the
    /// relation's columns at execution (`SubqueryScan` from the executed subquery,
    /// `CteScan` from the materialized rows), so a rename applied only to the parser's
    /// plan would reach the binding gate and `Describe` and not the rows.
    ctes: HashMap<String, CteEntry>,
    /// The CTEs a wrapping [`Plan::With`] materializes before the body, in the order
    /// they run: the **data-modifying** ones (`UPDATE/DELETE/INSERT … RETURNING`) —
    /// each runs once, regardless of whether the body references it (matching
    /// Postgres) — and the genuinely **recursive** ones of a `WITH RECURSIVE`. Read-only
    /// non-recursive CTEs are absent (they inline as `SubqueryScan`). Pushed as each CTE
    /// is resolved, so the order is declaration order under a plain `WITH` and the
    /// dependency order `parse_with_clause` sorts under `WITH RECURSIVE` (declaration
    /// order again for a list with a duplicate name, which is refused), and a later
    /// entry can read an earlier one. A data-modifying entry's alias list rides with it
    /// so the materializer renames the executed columns the same way the reference site
    /// renames the plan-side ones.
    materialized_ctes: Vec<crate::planner::CtePlan>,
    /// The analysis-order rank the next CTE registered takes ([`crate::planner::CtePlan::rank`]):
    /// the list's bodies are parsed in the order 18.6 analyses them, so a CTE's rank is
    /// the count of CTEs registered before it. Reset with [`Self::ctes`].
    next_cte_rank: u32,
    /// The name of the CTE whose `WITH RECURSIVE` body is being parsed — the one name a
    /// FROM reference counts as a **self**-reference in [`Self::recursive_ref_count`].
    /// Keyed on the name and not on the kind of the entry it resolves to: an earlier,
    /// already-resolved recursive CTE resolves to a `Plan::CteScan` too, and counting
    /// that as a self-reference refused every later CTE that read it.
    defining_cte: Option<String>,
    /// The `WITH RECURSIVE` names not yet resolved while the list's bodies are parsed in
    /// dependency order. Every name in the list is visible to every body (Postgres's
    /// scope rule for `RECURSIVE`), so a FROM reference to one of these must not fall
    /// through to a same-named base table: the dependency sort resolves a referenced
    /// CTE first, and a reference it did not see is refused rather than guessed.
    pending_ctes: Vec<String>,
    /// Incremented by `parse_table_factor` whenever a FROM factor names
    /// [`Self::defining_cte`] — the running self-reference count
    /// `parse_recursive_cte_body` reads. A **count** (not a flag): Postgres rejects a
    /// self-reference appearing more than once. Zeroed around each term's parse so the
    /// seed and recursive terms are measured independently; drives the degrade
    /// decision (0 → ordinary CTE).
    recursive_ref_count: usize,
    /// The statement's `WITH` list was written `WITH RECURSIVE`.
    with_recursive: bool,
    /// The base tables the statement's `FROM` items and DML targets name, in parse
    /// order, a read-only CTE's tables counted again at each reference that inlines it
    /// ([`Self::cte_tables`]). The part named by the statement body after a `WITH` list
    /// is what [`Self::wrap_with_ctes`] compares with a data-modifying entry's writes.
    touched_tables: Vec<String>,
    /// The base tables each CTE body named, by CTE name — what an inlined reference to
    /// a read-only CTE adds to [`Self::touched_tables`], since the reference reads them
    /// where it runs.
    cte_tables: HashMap<String, Vec<String>>,
    /// How many **expression** subqueries enclose the query currently being parsed —
    /// the `(SELECT …)`, `EXISTS (…)` and `IN (SELECT …)` forms, and only those.
    ///
    /// It is the **chain-completeness signal**: at depth `0` a query level's scope is
    /// the whole namespace its references may name, so the binding gate can close over
    /// it as it is built. At depth `> 0` the level sits inside an enclosing query whose
    /// relations are not in that scope yet — a correlated reference is ordinary SQL
    /// there — so the scope stays provisional until `resolve_correlated_subplans`
    /// re-runs it against the level-chained scope, which is where the gate fires.
    ///
    /// A **derived table** and a **CTE** reset it rather than incrementing, so a level
    /// inside one is treated as having its whole namespace. That is what makes the gate
    /// fire on those positions at all — counting them would leave every such level
    /// permanently provisional — and it **over-states Postgres's rule**, which is the
    /// cause of a recorded loud over-rejection class (`EDGES.md`).
    /// Without `LATERAL` such a FROM item is outside its **own level's**
    /// namespace — a sibling FROM relation is an error, not a binding — but the
    /// *enclosing levels* stay visible, exactly as they are to an expression subquery:
    /// measured on 18.6, `SELECT * FROM ct WHERE EXISTS (SELECT 1 FROM (SELECT 1 AS q
    /// ORDER BY ct.a) x)` answers rows and is `42P01` here. `planner::correlation`'s
    /// `Plan::SubqueryScan` arm states the rule in that form. The repair direction is to
    /// make a FROM item nested inside an expression subquery stay **provisional**, with
    /// the correlated re-walk judging its keys. The one-line form of that — resetting only
    /// when the depth is already `0` — **is refuted**: an adversarial probe measured ten
    /// statements turning 18.6-matching refusals into silently accepted wrong answers,
    /// eight of them writes, two destroying every row of a table. Leaving an item
    /// provisional is only safe where some re-walk will reach it with a **complete chain
    /// and a decidable namespace**, and neither holds generally — an `INSERT`'s clauses
    /// were reached by no re-walk at all (half-closed now, `planner::correlation`'s
    /// `Plan::Insert` arm), and a chain holding a non-exhaustively-registered relation
    /// answers `Uncertain`, so the re-walk declines (`EDGES.md`
    /// carries both routes).
    expr_subquery_depth: usize,
    /// How many **expression** subqueries enclose the FROM items (derived tables and CTE
    /// bodies) [`Self::expr_subquery_depth`] was reset for — the enclosing levels that
    /// reset hides.
    ///
    /// Read by one question only, a set operation's column typing: a reference to an
    /// enclosing level is still ordinary SQL inside such an item, so a set operation
    /// built there cannot type it and leaves the column pending for the correlated
    /// re-walk (`planner::query::OuterTyping`). The binding gate keeps reading the reset
    /// depth.
    from_item_enclosing_depth: usize,
    /// Whether the expression currently being read is under Postgres's restricted
    /// **`b_expr`** grammar rather than full `a_expr` — a *scoped mode*, not a
    /// binding-power threshold.
    ///
    /// `b_expr` is the production a *column constraint*'s `DEFAULT` takes
    /// (`gram.y`'s `ColConstraintElem: DEFAULT b_expr`, reached by `CREATE TABLE`
    /// and `ALTER TABLE … ADD COLUMN`). `ALTER TABLE … ALTER COLUMN … SET DEFAULT`
    /// is a different production over full `a_expr` (`alter_column_default`), so the
    /// two callers of `Parser::parse_column_default` do not share a grammar and
    /// using one for both is wrong for the other.
    ///
    /// `b_expr` is `c_expr` plus the arithmetic / `||` / comparison operators, `::`,
    /// and `IS [NOT] DISTINCT FROM` / `IS [NOT] DOCUMENT`; it excludes `AND` / `OR` /
    /// `NOT`, the `IS NULL`-family postfixes, `IN` / `BETWEEN` / `LIKE` / `ILIKE`,
    /// `AT TIME ZONE`, `COLLATE`, and the **quantified comparison**
    /// (`a_expr subquery_Op sub_type …` — `= ANY (…)` / `ALL` / `SOME`).
    ///
    /// **The exclusion list is the contract and every member needs an arm**; the
    /// quantified comparison is the one that was left out when this was written, and a
    /// list stated as "all four" cannot show a missing member. Re-derive it from
    /// `gram.y`'s `b_expr` block rather than from this sentence when adding an arm.
    /// It is **complete over what this parser can produce today**, which is a narrower
    /// claim than completeness against `gram.y`: `a_expr` also excludes `SIMILAR TO`
    /// (`gram.y:15976`), `row OVERLAPS row` (`:16058`), `IS NORMALIZED` (`:16260`) and
    /// the `IS json_predicate_type_constraint` family (`:16290`) from `b_expr`, and
    /// pglet's grammar has no arm for any of them — so each is `42601` on both engines
    /// by absence rather than by this mode. Adding one of those arms adds a member here.
    ///
    /// **`COLLATE` is on the list and is not this mode's to enforce.** In a column
    /// definition it is a `ColConstraint` element beside the `DEFAULT`, not part of the
    /// default's expression, so `DEFAULT 'x' COLLATE "C"` is *accepted* on 18.6 (its
    /// `column_default` reads `'x'::text`). pglet answers `42601` — to that and to the
    /// bare `v TEXT COLLATE "C"` alike, which is the control identifying it as the
    /// engine's pre-existing no-`COLLATE` gap rather than a `b_expr` verdict.
    ///
    /// **A binding-power threshold cannot express that, which is why this is a
    /// flag.** The excluded predicate band (`BP_PRED`) and `AT TIME ZONE` (`BP_AT`)
    /// bind *tighter* than the included comparisons (`BP_CMP`), so no single
    /// `min_bp` separates them, and a prefix production (`NOT`) is not gated by
    /// `min_bp` at all. The threshold that stood here before (`BP_B_EXPR`) therefore
    /// diverged from 18.6 in **both** directions, and every row below closes with the
    /// mode — each measured on 18.6, in both the column-constraint and the
    /// `SET DEFAULT` spelling:
    ///
    /// | spelling | 18.6 (column constraint) | threshold answered | mode answers |
    /// |---|---|---|---|
    /// | `DEFAULT 5 = 4` | accepted | `42601` | accepted |
    /// | `DEFAULT 5 IS DISTINCT FROM 4` | accepted | `42601` | accepted |
    /// | `DEFAULT NOT true` | `42601` | `0A000` | `42601` |
    /// | `DEFAULT now() AT TIME ZONE 'utc'` | `42601` | `0A000` | `42601` |
    /// | `DEFAULT 5 = ANY(ARRAY[1,2])` | `42601` | accepted | `42601` |
    ///
    /// The `SET DEFAULT` spelling of **every** row above is accepted on 18.6 (full
    /// `a_expr`) and is the control: the mode is off on that route, so nothing about
    /// them changes. The quantified row's "threshold answered" cell is *accepted*
    /// rather than a code because that arm had no mode test at all — it is the member
    /// the threshold never reached in either direction, and the reason this table is
    /// read as a list rather than counted.
    ///
    /// **A `(` resets to `a_expr`, on both engines**, and the reset is what
    /// [`Parser::parse_primary`] performs: every nested construct — a parenthesized
    /// group, a function argument, a `CASE` branch, an `ARRAY[…]` element — is read
    /// under full `a_expr`, and the mode is restored on the way out. So
    /// `DEFAULT 1 + (NOT true)::int` is accepted while
    /// `DEFAULT (now()) AT TIME ZONE 'utc'` and `DEFAULT (5 IS NULL) IS NULL` are
    /// `42601` — the postfix sits back at the `b_expr` level the paren returned to.
    /// All three measured on 18.6.
    ///
    /// The mode **propagates** through the productions `b_expr` defines recursively
    /// — a binary operator's right operand, a unary `+`/`-` operand, and an
    /// `IS DISTINCT FROM` right operand — because `gram.y` spells each of those
    /// `b_expr <op> b_expr`. Measured: `DEFAULT 1 = 1 AND true` is `42601`.
    b_expr: bool,
    /// The pending `SKIP LOCKED` / `NOWAIT` refusal, held until the whole statement has
    /// parsed. `parse_for_locking` records the first one it sees rather than raising on
    /// the spot, because 18.6 accepts both spellings and raises everything *else* first:
    /// its `42703`/`42P01` over the projection and FROM, and its `42601` over trailing
    /// junk, all beat a refusal pglet has and it does not. Deferring to [`parse`]'s
    /// per-statement boundary is what puts pglet's refusal last, where 18.6's absent one
    /// would be. The first refusal wins over a second (`… FOR UPDATE NOWAIT FOR SHARE
    /// SKIP LOCKED` reports `NOWAIT`), matching a left-to-right parse.
    lock_wait_policy_refusal: Option<Error>,
    /// The first error this parse reached that **18.6 raises in parse analysis** rather
    /// than in its raw parse, held until the whole statement has parsed
    /// ([`Self::owe_analysis_refusal`]). Two members today: the `42883` an operator
    /// pglet implements nothing for owes ([`Parser::owe_unknown_operator`]), and the
    /// `22003` a literal past `numeric`'s format cap owes (`Parser::number_literal`).
    ///
    /// Deferred because every error 18.6's raw parse raises — a grammar error anywhere
    /// in the statement — outranks one it raises in analysis; deferring to [`parse`]'s
    /// per-statement boundary is what puts it behind them. The **first** one wins,
    /// matching a left-to-right analysis: 18.6 transforms an expression's operands
    /// before resolving the operator over them, so a literal's `22003` inside an
    /// operand beats the enclosing operator's `42883`, and the parse reaches them in
    /// that order too.
    analysis_refusal: Option<Error>,
    /// An operator chain in this statement grew past the tree-depth limit and was
    /// replaced by a placeholder (`expr::OperatorChain`), so the statement owes `54001`
    /// — answered once the rest of it has parsed, because some errors 18.6 raises before
    /// parse analysis can still follow the chain. [`Self::settle`] decides which answers.
    depth_refusal: Option<Error>,
    /// Which statement of this parser's text owes [`Self::depth_refusal`] — the
    /// [`Self::statement_serial`] it was recorded under.
    depth_refusal_statement: u32,
    /// A count of the statements this parser has begun ([`Self::parse_statement`]), so
    /// an error can be told apart by the statement it belongs to. It matters where one
    /// parser reads several statements and a refusal owed by one can meet an error of
    /// another: a `DO` body.
    statement_serial: u32,
    /// The errors this parse raised that 18.6 raises **before the analysis that owes the
    /// `54001`** — in `gram.y`'s actions, or in `transformColumnDefinition` ahead of any
    /// expression's transform — each recorded where it is raised, with its origin
    /// ([`Self::before_analysis`], [`Self::before_statement_analysis`]). They are the only
    /// errors of a parse's own that outrank the `54001` a statement owes
    /// ([`Self::refusal_answer`]); the record is how that is decided by an error's
    /// origin rather than by its shape.
    before_analysis_errors: Vec<(ErrorFingerprint, ErrorOrigin)>,
    /// A `SKIP LOCKED` was read since the query tail's reading began — which 18.6 refuses
    /// beside a `WITH TIES` (`Parser::parse_query_tail`).
    saw_skip_locked: bool,
    /// The statement being parsed carries a row-locking clause somewhere — a `FOR
    /// UPDATE` / `NO KEY UPDATE` / `SHARE` / `KEY SHARE` item, not `FOR READ ONLY`.
    /// [`parse`] reads it at the statement's end and wraps a read in
    /// [`Plan::LockRows`], the fact a host takes the writer on.
    saw_locking_clause: bool,
    /// The clauses over the whole of the query `parse_select` parsed last, which a
    /// parenthesized query's clauses merge into (`select::QueryClauses::merge`).
    carried_clauses: select::QueryClauses,
    /// The `WITH` clause's deferred **duplicate-name** `42712`, fired by
    /// `parse_with_clause` once the whole list has parsed.
    ///
    /// Deferred because Postgres's `transformWithClause` scans the raw list after the
    /// statement parses and before any `analyzeCTE`, and pglet has no such split: raising
    /// at the declaration would put this ahead of a body's own **syntax** error, which
    /// Postgres's raw parse reports first. First duplicate wins.
    with_clause_verdict: Option<Error>,
    /// The `WITH` clause's deferred **arity** `42P10` ([`Error::CteColumnCountMismatch`]),
    /// outranked by [`Self::with_clause_verdict`] wherever both are pending — the order
    /// `transformWithClause`-then-`analyzeCTE` has, and the one a single first-wins slot
    /// cannot give, since the arity failure can be recorded by an earlier declaration
    /// than the duplicate.
    with_clause_arity_verdict: Option<Error>,
    /// The parameter types a client declared at `Parse`, indexed by `$N - 1`; `None`
    /// (or an index past the end) is a parameter left for inference. See
    /// [`parse_single_with_param_types`].
    declared_param_types: &'a [Option<TypeOid>],
    /// How many set operations this parse of the current statement has built — the
    /// ordinal [`Self::set_op_references`] is indexed by.
    set_ops_built: usize,
    /// The record of each set operation this parse built with a column pending an
    /// enclosing level, with its ordinal and the node **as built** — see
    /// [`Self::parse_statement`], which reads the node to give an orphaned record the
    /// hints of one this parse built equal to it.
    set_op_records: Vec<(usize, SetOpOuterTypes, Plan)>,
    /// On a statement's second parse, per set-operation ordinal, the enclosing-level
    /// reference types the first parse's re-walk recorded for it.
    set_op_references: Vec<Option<Vec<SideReferenceTypes>>>,
}

/// A `WITH` query registered in [`Parser::ctes`]: the body plan plus its `name(cols)`
/// column-alias list.
///
/// **One object per CTE relation.** The list is stored beside the plan, never applied
/// to it, because the plan is what both reference kinds *re-execute*: applying it here
/// would rename the plan-side names only. Every reader instead applies it through
/// [`crate::planner::alias_relation_columns`] — the read-only reference carries it on
/// the `Plan::SubqueryScan` it builds, the data-modifying one bakes it into the
/// `Plan::CteScan`'s `columns` **and** into the `Plan::With` entry the materializer
/// reads. Empty for a CTE written without a list, and for the recursive placeholder
/// (whose `columns` are renamed at construction).
struct CteEntry {
    /// The CTE body's plan — a `SELECT`, a data-modifying DML, or the recursive
    /// placeholder `Plan::CteScan`.
    plan: Plan,
    /// The `name(cols)` list, positionally replacing the body's own column names. Never
    /// longer than the body's column count: [`Parser::register_cte`] refuses a longer
    /// one `42P10` at `WITH` time.
    column_aliases: Vec<String>,
    /// The CTE's analysis-order rank ([`crate::planner::CtePlan::rank`]) — carried onto
    /// each [`Plan::SubqueryScan`] an ordinary read-only CTE inlines as. A recursive
    /// CTE's placeholder holds the rank the CTE takes once registered: no other CTE is
    /// registered while its body is parsed.
    rank: u32,
}

impl<'a> Parser<'a> {
    /// Create a parser over `source` with the given `schema` context.
    ///
    /// **A sub-parser's deferred state does not flow out with its plan.** Some fields
    /// here are *obligations* rather than results — [`Self::lock_wait_policy_refusal`]
    /// is one — and [`parse`] discharges them off the parser it created, so a nested
    /// `Parser::new` that returns only a `Plan` silently drops whatever its body
    /// recorded. The DO block (`parser::plpgsql`) is the tree's one such site today,
    /// and it did drop one; it now calls [`Self::adopt_deferred_refusals`]. Any new
    /// sub-parser must do the same — this is a discipline the compiler does not
    /// enforce, which is why it is written at the constructor rather than only at the
    /// call site.
    fn new(source: &'a str, schema: &'a Schema) -> Result<Self> {
        Ok(Self {
            ts: TokenStream::new(source)?,
            schema,
            depth: 0,
            ctes: HashMap::new(),
            materialized_ctes: Vec::new(),
            next_cte_rank: 0,
            defining_cte: None,
            pending_ctes: Vec::new(),
            recursive_ref_count: 0,
            with_recursive: false,
            touched_tables: Vec::new(),
            cte_tables: HashMap::new(),
            expr_subquery_depth: 0,
            from_item_enclosing_depth: 0,
            b_expr: false,
            lock_wait_policy_refusal: None,
            analysis_refusal: None,
            depth_refusal: None,
            depth_refusal_statement: 0,
            statement_serial: 0,
            before_analysis_errors: Vec::new(),
            saw_skip_locked: false,
            saw_locking_clause: false,
            carried_clauses: select::QueryClauses::default(),
            with_clause_verdict: None,
            with_clause_arity_verdict: None,
            declared_param_types: &[],
            set_ops_built: 0,
            set_op_records: Vec::new(),
            set_op_references: Vec::new(),
        })
    }

    /// [`build_set_op`] at the current nesting, typed as this parse of the statement
    /// types it — see [`Self::parse_statement`].
    fn build_set_op(&mut self, op: SetOpKind, all: bool, left: Plan, right: Plan) -> Result<Plan> {
        let ordinal = self.set_ops_built;
        self.set_ops_built += 1;
        let references = self.set_op_references.get(ordinal).and_then(Option::as_ref);
        let typing = match references {
            _ if self.no_enclosing_level() => SetOpTyping::Complete,
            Some(references) => SetOpTyping::Resolved(references),
            None => SetOpTyping::Deferring,
        };
        let plan = build_set_op(op, all, left, right, self.schema, typing)?;
        if let Plan::SetOp {
            outer_types: Some(record),
            ..
        } = &plan
        {
            self.set_op_records
                .push((ordinal, record.clone(), crate::depth::clone_plan(&plan)?));
        }
        Ok(plan)
    }

    /// Adopt every **deferred refusal** a sub-parser recorded, so it is raised at this
    /// parser's statement boundary instead of being dropped with the sub-parser.
    ///
    /// The counterpart to [`Self::new`]'s note. Today that is
    /// [`Self::lock_wait_policy_refusal`], [`Self::analysis_refusal`] and
    /// [`Self::depth_refusal`], and this is the only place that enumeration lives.
    ///
    /// **The test for membership is where a field is discharged, not that it defers.**
    /// This function exists for state whose fire point is the **statement** boundary —
    /// [`parse`] raises `lock_wait_policy_refusal` after the statement's trailing token
    /// check, so a sub-parser's copy would never be seen. [`Self::with_clause_verdict`]
    /// and [`Self::with_clause_arity_verdict`] defer too and deliberately do **not**
    /// belong here: `parse_with_clause` fires them at the end of its own comma list, so
    /// a sub-parser that parses a `WITH` clause has already discharged them before its
    /// plan comes back, and adopting them would fire a verdict a second time against an
    /// outer statement that never had the duplicate.
    ///
    /// The outer parser's own refusal wins if it already has one — it was recorded
    /// earlier in the same left-to-right parse.
    fn adopt_deferred_refusals(&mut self, sub: &mut Parser<'_>) {
        if let Some(refusal) = sub.lock_wait_policy_refusal.take() {
            self.lock_wait_policy_refusal.get_or_insert(refusal);
        }
        if let Some(refusal) = sub.analysis_refusal.take() {
            self.analysis_refusal.get_or_insert(refusal);
        }
        if sub.depth_refusal.take().is_some() {
            self.owe_depth_refusal();
        }
    }

    /// Record that the statement being parsed owes `54001` ([`Self::depth_refusal`]),
    /// unless a refusal is already owed — the first one, left to right, is the one owed.
    fn owe_depth_refusal(&mut self) {
        if self.depth_refusal.is_none() {
            self.depth_refusal = Some(Error::StackDepthExceeded);
            self.depth_refusal_statement = self.statement_serial;
        }
    }

    /// Parse `f` as the body of an **expression** subquery — one level deeper on
    /// [`Self::expr_subquery_depth`], restored however `f` returns.
    fn in_expr_subquery<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        self.expr_subquery_depth += 1;
        let out = f(self);
        self.expr_subquery_depth -= 1;
        out
    }

    /// Parse `f` as the body of a **derived table** or **CTE** — a FROM item, whose
    /// namespace does not include its *own* level's other FROM relations, so the depth
    /// resets to `0` rather than deepening. The reset treats such a level as complete,
    /// which is wider than Postgres's rule (the enclosing *levels* are still visible) and
    /// is a recorded over-rejection; see [`Self::expr_subquery_depth`].
    fn in_from_item<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T>) -> Result<T> {
        let saved = std::mem::take(&mut self.expr_subquery_depth);
        self.from_item_enclosing_depth += saved;
        let out = f(self);
        self.from_item_enclosing_depth -= saved;
        self.expr_subquery_depth = saved;
        out
    }

    /// Whether no query level encloses the one being parsed — [`Self::scope_is_complete`]
    /// with the enclosing levels a FROM item's reset hides counted back in. A set
    /// operation's column typing reads it; see [`Self::from_item_enclosing_depth`].
    const fn no_enclosing_level(&self) -> bool {
        self.expr_subquery_depth == 0 && self.from_item_enclosing_depth == 0
    }

    /// Whether the query level being parsed right now has its **whole** namespace in
    /// the scope built for it — true exactly at expression-subquery depth `0`.
    const fn scope_is_complete(&self) -> bool {
        self.expr_subquery_depth == 0
    }

    /// Whether the current token is a bare identifier whose text equals `word`
    /// (case-insensitively). Used for contextual keywords that aren't in the
    /// reserved table — `enum`, `restart`, `identity`, `time`/`zone` among them —
    /// so they stay usable as ordinary identifiers (column/table names). An index
    /// method (`gin`/`btree`) is **not** one of these: `USING <method>` reads a
    /// name through `expect_name`, so a quoted `USING "GIN"` names no method.
    fn at_ident(&self, word: &str) -> bool {
        matches!(self.ts.peek_kind(), TokenKind::Identifier)
            && self.ts.peek_text().eq_ignore_ascii_case(word)
    }

    /// Consume the current token iff it is the contextual identifier `word`.
    fn eat_ident(&mut self, word: &str) -> Result<bool> {
        if self.at_ident(word) {
            self.ts.bump()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Consume the contextual identifier `word`, or a `42601` error.
    fn expect_ident(&mut self, word: &str) -> Result<()> {
        if self.at_ident(word) {
            self.ts.bump()
        } else {
            Err(self.ts.expected(word))
        }
    }

    /// Enter one nesting level, failing loud past [`MAX_DEPTH`] — or past the thread's
    /// tree-depth limit where that is lower ([`crate::depth`]), which charges nesting as
    /// levels: the descent's own frames are what a small stack runs out of first
    /// (without this, 50 nested scalar subqueries overflowed a 2 MiB debug thread on the
    /// way down, well inside [`MAX_DEPTH`] — measured).
    fn enter(&mut self) -> Result<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.ts.error("expression nesting too deep"));
        }
        crate::depth::check(self.depth)?;
        crate::depth::check_stack()
    }

    /// Leave one nesting level.
    const fn leave(&mut self) {
        self.depth -= 1;
    }

    /// Parse a **non-empty** comma-separated list `f [, f]*`, consuming each
    /// separating `,` and stopping at the first token that isn't one.
    ///
    /// The caller owns any surrounding delimiters (parens/brackets). This is the
    /// `one-or-more` shape — SET assignments, `VALUES` tuples, a parenthesized name
    /// list, `ON CONFLICT` keys — so the element closure carries any per-item special
    /// (a `DEFAULT` keyword, an opclass). The token sequence is identical to the
    /// hand-rolled `loop { push(f()?); if !eat(Comma) { break } }` it replaces.
    fn comma_list<T>(
        &mut self,
        mut parse_one: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut items = Vec::new();
        loop {
            items.push(parse_one(self)?);
            if !self.ts.eat(&TokenKind::Comma)? {
                break;
            }
        }
        Ok(items)
    }

    /// Parse a **possibly-empty** comma-separated list, stopping before `close`
    /// (which is **not** consumed — the caller closes the bracket).
    ///
    /// When the cursor is already at `close` the list is empty (`vec![]`); otherwise
    /// it is one-or-more via [`comma_list`](Self::comma_list). The `zero-or-more`
    /// shape — `IN (…)`, `ARRAY[…]`, a function-call argument list, an `unnest` array.
    fn comma_list_until<T>(
        &mut self,
        close: &TokenKind,
        parse_one: impl FnMut(&mut Self) -> Result<T>,
    ) -> Result<Vec<T>> {
        if self.ts.at(close) {
            return Ok(Vec::new());
        }
        self.comma_list(parse_one)
    }
}

/// Parse a single SQL expression into the planner [`Expr`] IR.
///
/// `schema` is consulted only for embedded subqueries and `CAST`-to-user-`ENUM`;
/// pass an empty [`Schema`] for a schema-free expression.
///
/// # Errors
///
/// Returns the crate [`Error`] — `42601` (syntax), `42704` (unsupported type), or
/// `0A000` (unsupported construct) — on any failure, including leftover input
/// after the expression.
pub fn parse_expr(sql: &str, schema: &Schema) -> Result<Expr> {
    let mut parser = Parser::new(sql, schema)?;
    let expr = parser.parse_expr_bp(0);
    let expr = parser.settle(expr)?;
    // a trailing `;` is allowed
    let end = parser.ts.eat(&TokenKind::Semicolon).and_then(|_| {
        if parser.ts.at_eof() {
            Ok(())
        } else {
            Err(parser.ts.expected("end of input"))
        }
    });
    parser.settle(end)?;
    if parser.depth_refusal.is_some() {
        return Err(parser.refusal_answer(None));
    }
    parser.analysis_refusal.take().map_or(Ok(expr), Err)
}

/// Parse a sequence of `;`-separated SQL statements into planner [`Plan`]s.
///
/// Supported: `SELECT`, `INSERT` (including the expression / partial `ON CONFLICT`
/// target and `DO UPDATE … WHERE`), `UPDATE`, `DELETE`, the DDL statements
/// (`CREATE`/`ALTER`/`DROP`/`TRUNCATE`), the transaction-control statements
/// (`BEGIN`/`COMMIT`/`ROLLBACK`), and a minimal `DO $$ … $$` block (see
/// [`plpgsql`]). An unrecognized statement kind is a loud `NotImplemented`.
/// `schema` is consulted for FROM-table resolution, WHERE type-resolution, INSERT
/// column mapping / coercion, assignment coercion, subqueries, and enum casts.
///
/// # Errors
///
/// Returns the crate [`Error`] on any syntax error or unsupported construct.
pub fn parse(sql: &str, schema: &Schema) -> Result<Vec<Plan>> {
    parse_with(Parser::new(sql, schema)?)
}

/// The statement loop behind [`parse`] and [`parse_single_with_param_types`].
fn parse_with(mut parser: Parser<'_>) -> Result<Vec<Plan>> {
    let mut plans = Vec::new();
    loop {
        while parser.ts.eat(&TokenKind::Semicolon)? {} // skip empty statements
        if parser.ts.at_eof() {
            break;
        }
        parser.before_analysis_errors.clear();
        parser.saw_locking_clause = false;
        let plan = parser.parse_statement();
        let plan = parser.settle(plan)?;
        // The statement as a whole, once: every construct that deepens a tree is
        // measured where it is built, and this is the bound on what the executor,
        // the renderers and the IR's own `Drop`/`Clone` will walk ([`crate::depth`]).
        // Skipped for a statement already owing the refusal: its chain is a placeholder.
        if parser.depth_refusal.is_none() {
            let measured = crate::depth::check(crate::depth::plan_depth(&plan));
            parser.settle(measured)?;
        }
        let end = parser.ts.eat(&TokenKind::Semicolon).and_then(|ended| {
            if ended || parser.ts.at_eof() {
                Ok(())
            } else {
                Err(parser.ts.expected("';' or end of input"))
            }
        });
        parser.settle(end)?;
        if parser.depth_refusal.is_some() {
            return Err(parser.refusal_answer(None));
        }
        // The statement has parsed and its trailing token boundary has been checked:
        // this is where the deferred refusals fire, so every error 18.6 raises in its
        // raw parse — `42601` from a grammar it accepts nothing of, trailing junk
        // included — wins over a refusal it raises in analysis, or does not raise at all.
        //
        // An operator pglet implements nothing for is 18.6's own `42883`, raised where
        // it resolves the operator; the `SKIP LOCKED` / `NOWAIT` refusal is pglet's own,
        // with no 18.6 counterpart, so it goes last of all — behind the `42703`/`42P01`
        // the projection and FROM raise.
        if let Some(refusal) = parser.analysis_refusal.take() {
            return Err(refusal);
        }
        if let Some(refusal) = parser.lock_wait_policy_refusal.take() {
            return Err(refusal);
        }
        // A locking clause over a read is the one fact of it a host acts on; a statement
        // that writes takes the writer already.
        let plan = if parser.saw_locking_clause && !plan.mutates_storage() {
            Plan::LockRows {
                input: Box::new(plan),
            }
        } else {
            plan
        };
        plans.push(plan);
    }
    Ok(plans)
}

/// An error's identity for [`Parser::before_analysis_errors`]: its SQLSTATE, its
/// message and its position — what a client can tell two errors apart by.
type ErrorFingerprint = (&'static str, String, Option<u32>);

/// Where in 18.6's front end an error recorded by [`Parser::before_analysis_errors`] is
/// raised, which decides whose `54001` it outranks.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ErrorOrigin {
    /// The raw parse (`gram.y`'s actions) — which 18.6 runs over the whole text before
    /// analysing any statement of it, a `DO` body's statements included (PL/pgSQL
    /// raw-parses each one as it compiles the block). It outranks a refusal owed by any
    /// statement of the text.
    RawParse,
    /// Parse analysis of the statement with this [`Parser::statement_serial`], ahead of
    /// any expression of that statement. It outranks only that statement's own refusal:
    /// an earlier statement's is raised when that statement is analysed, first.
    Analysis(u32),
}

fn fingerprint(error: &Error) -> ErrorFingerprint {
    (error.sqlstate(), error.to_string(), error.position())
}

/// Why 18.6's raw parse of a text fails, as far as a scan of its tokens can tell
/// without a grammar ([`raw_parse_failure`]).
enum RawParseFailure {
    /// The scanner refuses the text: the first lexical error in it.
    Lexical(Error),
    /// A bracket closes nothing, closes the other kind, or is open at a `;` or at the
    /// end of the text.
    Brackets,
}

/// Whether 18.6's **raw parse** of `source` certainly fails, and why — decided without
/// a grammar, from two facts about 18.6's front end:
///
/// - its scanner (`scan.l`) runs inside the raw parse, so a text it refuses is refused
///   before any analysis. pglet's lexer refuses what that scanner refuses — an
///   unterminated literal or comment, a malformed escape, trailing junk, a character no
///   token begins with — and **nothing else**: every `op_chars` spelling it lexes too
///   (`crate::lexer::TokenKind::Op`), so a lexical error here is one 18.6's scanner
///   raises as well;
/// - every alternative of every rule in `gram.y` is bracket-balanced — each `'('` and
///   `'['` it names is closed by the matching `')'` / `']'` in the same alternative,
///   properly nested — so a text whose `(`/`)`/`[`/`]` tokens do not nest, or leave
///   one open at a statement's `;` or at the end, derives from no statement. (Checked
///   over all 3 606 alternatives of the PostgreSQL source's `gram.y`; the brackets are
///   self-characters in `scan.l`, never part of an operator, as they are in pglet's
///   lexer.) The brackets read are pglet's tokens, which are `scan.l`'s — a `--`
///   comment included, which ends at a line feed or a carriage return on both.
///
/// 18.6 raw-parses a query string whole — every statement in it, before analysing any —
/// so this scans the whole text. The two failures are reported in text order. Where
/// the scanner's error is not a `42601` and 18.6's grammar stops earlier, 18.6 answers
/// the grammar's `42601` where this answers the scanner's; nothing here can see which.
fn raw_parse_failure(source: &str) -> Option<RawParseFailure> {
    let mut lexer = Lexer::new(source);
    let mut open = Vec::new();
    loop {
        let token = match lexer.next_token() {
            Ok(token) => token,
            Err(e) => return Some(RawParseFailure::Lexical(lex_error_to_error(e, source))),
        };
        match token.kind {
            TokenKind::LParen | TokenKind::LBracket => open.push(token.kind),
            TokenKind::RParen | TokenKind::RBracket => {
                let opener = if token.kind == TokenKind::RParen {
                    TokenKind::LParen
                } else {
                    TokenKind::LBracket
                };
                if open.pop() != Some(opener) {
                    return Some(RawParseFailure::Brackets);
                }
            }
            TokenKind::Semicolon | TokenKind::Eof if !open.is_empty() => {
                return Some(RawParseFailure::Brackets);
            }
            TokenKind::Eof => return None,
            _ => {}
        }
    }
}

impl Parser<'_> {
    /// Record `error` as one 18.6 raises **before parse analysis**, and hand it back to
    /// be raised. A call site vouches for two things: that 18.6 raises this error from its
    /// raw grammar (`gram.y`'s actions) or ahead of transforming any expression of the
    /// statement, and that pglet raises it under exactly 18.6's condition — so the error
    /// outranks the `54001` a too-deep chain earlier in the statement owes
    /// ([`Self::refusal_answer`]).
    fn before_analysis(&mut self, error: Error) -> Error {
        self.before_analysis_errors
            .push((fingerprint(&error), ErrorOrigin::RawParse));
        error
    }

    /// Record `error` as the statement's deferred **parse-analysis** refusal, unless one
    /// is already owed ([`Self::analysis_refusal`] is first-wins). The caller parses on
    /// past it, so the rest of the statement is read as 18.6 reads it and a grammar
    /// error anywhere in it still outranks this.
    fn owe_analysis_refusal(&mut self, error: Error) {
        self.analysis_refusal.get_or_insert(error);
    }

    /// [`Self::before_analysis`] for an error 18.6 raises in its **parse analysis** of
    /// the statement, ahead of transforming any of its expressions —
    /// `transformColumnDefinition`'s. It outranks the `54001` the statement itself owes,
    /// and not one an earlier statement of the same `DO` body owes: 18.6 runs a body's
    /// statements one after another, and analyses the second only once the first has
    /// run (`DO $$ BEGIN INSERT INTO t VALUES (<chain>); CREATE TABLE r (x int DEFAULT 1
    /// DEFAULT 2); END $$` is `54001` there).
    fn before_statement_analysis(&mut self, error: Error) -> Error {
        let origin = ErrorOrigin::Analysis(self.statement_serial);
        self.before_analysis_errors
            .push((fingerprint(&error), origin));
        error
    }

    /// [`reduce_qualified_name`] for a name 18.6's **raw parse** refuses at four parts —
    /// a `qualified_name` (a `FROM` item, a statement's target, a `REFERENCES` table),
    /// whose `gram.y` action is `makeRangeVarFromQualifiedName`, or a single `ColId`,
    /// which no dotted name derives ([`Self::parse_relation_name`]).
    ///
    /// That four-part refusal is recorded as outranking the refusal of a too-deep chain
    /// ([`Self::before_analysis`]). At three parts it is pglet's own refusal — 18.6
    /// resolves `db.schema.table` in analysis — and is not. A position `gram.y` reads as
    /// an `any_name` does not come here: 18.6 refuses its four-part name only when the
    /// statement runs ([`Self::parse_object_name`]), and a `SEQUENCE NAME`'s in the
    /// statement's analysis ([`Self::before_statement_analysis`]).
    fn reduce_grammar_name(&mut self, parts: &[String]) -> Result<String> {
        reduce_qualified_name(parts).map_err(|e| {
            if parts.len() >= 4 {
                self.before_analysis(e)
            } else {
                e
            }
        })
    }

    /// Settle a step of the parse against the statement's depth refusal: an error it
    /// raised is answered by [`Self::refusal_answer`] when the statement owes the
    /// refusal — a chain replaced by a placeholder ([`Self::depth_refusal`]) — or when
    /// the error *is* a refusal, raised on the spot by a bound that does not defer.
    fn settle<T>(&self, result: Result<T>) -> Result<T> {
        match result {
            Err(e) if self.depth_refusal.is_some() || matches!(e, Error::StackDepthExceeded) => {
                Err(self.refusal_answer(Some(e)))
            }
            other => other,
        }
    }

    /// Whether `error` was recorded as raised ahead of the analysis that owes this
    /// parser's refusal ([`Self::before_analysis_errors`]).
    fn outranks_refusal(&self, error: &Error) -> bool {
        let print = fingerprint(error);
        self.before_analysis_errors
            .iter()
            .any(|(recorded, origin)| {
                *recorded == print
                    && match origin {
                        ErrorOrigin::RawParse => true,
                        ErrorOrigin::Analysis(statement) => {
                            *statement == self.depth_refusal_statement
                        }
                    }
            })
    }

    /// The answer of a statement that owes `54001` — `error` being what its parse raised
    /// after the debt, if anything.
    ///
    /// 18.6 raises `54001` from `check_stack_depth` in parse analysis (or later), never
    /// in its raw parse, so it is outranked by exactly the errors 18.6 raises before the
    /// point in analysis where that statement's too-deep tree is transformed. pglet
    /// answers the two kinds of those it can attribute:
    ///
    /// - an error raised at a site that records its origin ([`Self::before_analysis`]);
    /// - a raw parse that certainly fails ([`raw_parse_failure`]) — the scanner's error,
    ///   or, for brackets, `error` when it is a `42601` and a `42601` of pglet's own when
    ///   it is not.
    ///
    /// Everything else answers `54001`: pglet's grammar is narrower than 18.6's, so a
    /// `42601` pglet raises may be one 18.6's grammar accepts — `LIMIT 1+1`, a nesting
    /// past pglet's wall, an unsupported predicate — and nothing about the error tells
    /// the two apart; and an analysis error (`42703`, `42P01`, `42883`, …) may be the
    /// placeholder's own, or one 18.6 raises only after the refusal. Where 18.6 answers
    /// an error pglet cannot attribute, `EDGES.md` row 171 records it.
    fn refusal_answer(&self, error: Option<Error>) -> Error {
        let error = match error {
            Some(e) if self.outranks_refusal(&e) => return e,
            other => other,
        };
        match raw_parse_failure(self.ts.source()) {
            Some(RawParseFailure::Lexical(e)) => e,
            Some(RawParseFailure::Brackets) => match error {
                Some(e @ Error::Syntax { .. }) => e,
                _ => Error::syntax("unbalanced parentheses or brackets"),
            },
            None => Error::StackDepthExceeded,
        }
    }
}

/// Split `sql` into its top-level statement substrings (trimmed, no trailing `;`).
///
/// Uses the lexer so a `;` inside a string / comment / dollar-quoted body doesn't
/// split. Returned slices borrow `sql`, so each statement can be parsed+planned
/// independently against the live schema — preserving the per-statement-replan
/// model the executors use (an earlier statement's DDL is visible to a later one).
///
/// A lexical error fails the whole text, as 18.6's raw parse of a query string fails it
/// before running any statement of it — and pglet's lexer raises one only where 18.6's
/// scanner does, every operator spelling being a token here too
/// (`crate::lexer::TokenKind::Op`). An operator pglet implements nothing for is the
/// holding statement's own `42883` at its analysis, so the pieces before it run as
/// pglet runs the pieces before any failing statement (`EDGES.md` rows 68 and 175).
///
/// # Errors
///
/// Returns the lexical error (`42601`, or `22021` / `22025` for an `E'…'` literal) if
/// the input fails 18.6's scanner as well as pglet's.
pub fn split_statements(sql: &str) -> Result<Vec<&str>> {
    split_pieces(sql, false)
}

/// [`split_statements`] less every piece that holds no token — the statements 18.6's
/// raw parse **counts**.
///
/// A piece of comments and whitespace alone (`SELECT 1; -- c`, `SELECT 1; /* c */`)
/// derives no statement in `gram.y` (`stmtmulti` drops an empty `stmt`), so a caller
/// that asks *how many statements* — `exec_parse_message`'s `cannot insert multiple
/// commands into a prepared statement`, or a single-statement sub-protocol — must not
/// count it. [`split_statements`] keeps such a piece because the simple-query loop runs
/// every piece it is handed, and what an empty one answers there is that loop's own
/// divergence (`EDGES.md` row 18).
///
/// # Errors
///
/// As [`split_statements`].
pub fn raw_statements(sql: &str) -> Result<Vec<&str>> {
    split_pieces(sql, true)
}

/// The splitter behind [`split_statements`] and [`raw_statements`]: `tokens_only`
/// drops a piece in which the lexer found no token.
fn split_pieces(sql: &str, tokens_only: bool) -> Result<Vec<&str>> {
    let mut lexer = Lexer::new(sql);
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut has_token = false;
    loop {
        let tok = match lexer.next_token() {
            Ok(tok) => tok,
            Err(e) => return Err(lex_error_to_error(e, sql)),
        };
        match tok.kind {
            TokenKind::Semicolon | TokenKind::Eof => {
                let end = if tok.kind == TokenKind::Eof {
                    sql.len()
                } else {
                    tok.start
                };
                let piece = crate::lexer::trim_space(&sql[start..end]);
                if !piece.is_empty() && (has_token || !tokens_only) {
                    out.push(piece);
                }
                if tok.kind == TokenKind::Eof {
                    return Ok(out);
                }
                start = tok.end;
                has_token = false;
            }
            _ => has_token = true,
        }
    }
}

/// Parse a single SQL statement into its [`Plan`], or `None` if `sql` is empty
/// (whitespace / a lone `;`). Errors if `sql` holds more than one statement.
///
/// The single-statement entry point for the executors' per-statement loop (paired
/// with [`split_statements`]) and for the single-query extended-protocol path.
///
/// # Errors
///
/// Returns the crate [`Error`] on a syntax error, an unsupported construct, or more
/// than one statement.
pub fn parse_single(sql: &str, schema: &Schema) -> Result<Option<Plan>> {
    parse_single_with_param_types(sql, schema, &[])
}

/// [`parse_single`] for a statement whose parameter types a client declared at `Parse`.
///
/// `declared` is indexed by `$N - 1`, and a `Some` entry gives that parameter its type
/// **for parse analysis**, as 18.6's `exec_parse_message` does: the Parse message's
/// OIDs seed the parameter-type array `variable_paramref_hook` reads, so a declared
/// `$N` is a `Param` of that type from its first reference, and the operators,
/// functions, common-type resolutions and result columns over it proceed from the
/// declared type. pglet reaches the same place by reading such a `$N` as
/// `Expr::Cast { Parameter, <type> }` where the parser reads the token — the shape a
/// written `$N::<type>` lowers to, so the coercion pass, the gates and both
/// declared-type walkers see a typed leaf through the `Cast` arms they already carry.
/// The bound value is decoded as the declared type, so the cast converts nothing.
///
/// A `None` entry, and a `$N` past the end of `declared`, is read as a bare
/// `Expr::Parameter`, as [`parse_single`] reads it, and keeps its inference. A declared
/// OID `TypeOid` does not model arrives here as `None` — the wire decoder collapses it
/// (`docs/param_binding.md`).
///
/// # Errors
///
/// As [`parse_single`].
pub fn parse_single_with_param_types(
    sql: &str,
    schema: &Schema,
    declared: &[Option<TypeOid>],
) -> Result<Option<Plan>> {
    let mut parser = Parser::new(sql, schema)?;
    parser.declared_param_types = declared;
    let mut plans = match parse_with(parser) {
        // 18.6's `exec_parse_message` counts the statements after its raw parse and
        // before analysing any, so a refusal — raised in analysis there — is outranked
        // by the count. The raw parse has already outranked it where it can
        // ([`Parser::refusal_answer`]), and so a split here cannot fail: the splitter
        // fails only on a scanner error, which that raw-parse test has answered first.
        Err(Error::StackDepthExceeded) => {
            return Err(match raw_statements(sql) {
                Ok(statements) if statements.len() > 1 => {
                    Error::syntax("expected a single SQL statement".to_string())
                }
                Ok(_) => Error::StackDepthExceeded,
                Err(e) => e,
            });
        }
        plans => plans?,
    };
    match plans.len() {
        0 => Ok(None),
        1 => Ok(Some(plans.remove(0))),
        _ => Err(Error::syntax("expected a single SQL statement".to_string())),
    }
}

impl Parser<'_> {
    /// Parse a single statement into a [`Plan`], **twice** when a set operation of it was
    /// built with a column pending an enclosing query level.
    ///
    /// Such a column carries a provisional type until the correlated re-walk unifies it,
    /// and every reader that asks the node for its columns before then — an enclosing set
    /// operation, a derived table and a scalar subquery's common type were each measured
    /// to keep it — is handed that type. Rather than making each
    /// reader wait, the first parse's re-walk records what it unified each column with
    /// ([`SetOpOuterTypes`], whose handle this parser keeps in [`Self::set_op_records`]),
    /// and the statement is parsed again from the same token checkpoint with those types
    /// ([`SetOpTyping::Resolved`]). The returned plan is the second parse's, and it holds
    /// no pending column: a column the re-walk never unified leaves one pending there too,
    /// and that plan is refused rather than returned. `docs/scope.md`
    /// § A pending set-op column's type is never read.
    ///
    /// A leading `WITH` populates the statement-scoped CTE context (consumed during
    /// the body parse, then cleared) — the CTEs are visible only to the statement
    /// body that follows. When any CTE is **data-modifying**
    /// (`UPDATE/DELETE/INSERT … RETURNING`) or genuinely **recursive**, the body is
    /// wrapped in a [`Plan::With`] that materializes those CTEs once before executing it.
    fn parse_statement(&mut self) -> Result<Plan> {
        self.statement_serial += 1;
        let start = self.ts.checkpoint();
        self.set_ops_built = 0;
        self.set_op_records.clear();
        self.set_op_references.clear();
        let plan = self.parse_statement_once()?;
        if self.set_op_records.is_empty() {
            return Ok(plan);
        }
        // A set operation was built with a column pending an enclosing level, so some
        // reader may have typed an expression, a common type, an enclosing set operation
        // or a derived table's column from its provisional type. The first parse has
        // succeeded, so the correlated re-walk has unified every such column it reached
        // and recorded what with: parse the statement again with those types, building
        // each node complete. No reader of the plan this returns sees a provisional type:
        // a node the second parse still leaves pending is refused below rather than
        // returned.
        //
        // A node the re-walk never *visited* has to hint all the same, or the second parse
        // builds it pending while its siblings are built complete and its own gates read
        // the mixed plan — a `SELECT DISTINCT`'s sort key, built pending, no longer equals
        // the target entry it was equal to in the first parse, and the match that answered
        // becomes `42P10`. Such a node was discarded *because* it equalled another, so its
        // hints are that one's ([`Self::hint_orphaned_records`]).
        let records = std::mem::take(&mut self.set_op_records);
        let mut references = Vec::new();
        for (ordinal, record, _) in &records {
            if references.len() <= *ordinal {
                references.resize(*ordinal + 1, None);
            }
            references[*ordinal] = record.hints();
        }
        Self::hint_orphaned_records(&records, &mut references);
        drop(records);
        drop(plan);
        self.ts.restore(start);
        self.lock_wait_policy_refusal = None;
        self.analysis_refusal = None;
        self.depth_refusal = None;
        self.set_ops_built = 0;
        self.set_op_references = references;
        let plan = self.parse_statement_once();
        self.set_op_references.clear();
        let unresolved = std::mem::take(&mut self.set_op_records);
        let plan = plan?;
        // A column the first parse's re-walk never unified has no recorded type, so the
        // second parse left it pending again, and a plan holding it would hand its
        // provisional type to every reader. Refused rather than returned.
        if !unresolved.is_empty() {
            return Err(Self::pending_set_op_column());
        }
        Ok(plan)
    }

    /// Give each record the correlated re-walk never reached the hints of a node this
    /// parse built **equal** to it.
    ///
    /// The re-walk visits the plan, so a node an expression held and a later rewrite then
    /// discarded is never unified: `SELECT DISTINCT (SELECT i EXCEPT SELECT '7') FROM t
    /// ORDER BY (SELECT i EXCEPT SELECT '7')` matches the sort key to the target entry it
    /// equals and keeps only the entry, leaving the key's own record with nothing. Its
    /// hints are the entry's — the two nodes were built equal, which is *why* the match
    /// took one for the other — and without them the second parse builds the two
    /// differently and the match misses.
    ///
    /// The nodes compared are the ones **as built**, before the re-walk wrote its unified
    /// types into them, so this is build-time identity rather than a comparison of what
    /// each became — which is a **weaker** question than whether two nodes are the same
    /// expression, because `Plan` equality is blind to the enclosing level a correlated
    /// name resolves at. Two subqueries spelled alike whose bare column binds at different
    /// levels are equal here, and the `find` below takes the **first** of them in build
    /// order, so which record an orphan borrows from depends on the order of the target
    /// list. Measured: over `pp(i INT)` and `qq(i TEXT)`, one order answers as 18.6 does
    /// and the swapped one is `42P10` where 18.6 still answers — 18.6 answers **both**, so
    /// the repair is a match that carries the level, not a second refusal
    /// (`docs/scope.md`, `EDGES.md` row 153).
    ///
    /// What a record that stays unhinted takes is **not**
    /// [`Self::pending_set_op_column`]: the second parse builds the orphan's node unlike
    /// the entry it must match, the match misses, the node survives into the plan, and its
    /// clause's own gate raises — an error [`Self::parse_statement`] returns before it
    /// reaches the `unresolved` check. So what this moves is that error. Every wrong borrow
    /// measured costs a refusal rather than an answer, and nothing here bounds it to that.
    fn hint_orphaned_records(
        records: &[(usize, SetOpOuterTypes, Plan)],
        references: &mut [Option<Vec<SideReferenceTypes>>],
    ) {
        let hinted: Vec<(&Plan, usize)> = records
            .iter()
            .filter(|(ordinal, ..)| references[*ordinal].is_some())
            .map(|(ordinal, _, plan)| (plan, *ordinal))
            .collect();
        if hinted.is_empty() {
            return;
        }
        for (ordinal, _, plan) in records {
            if references[*ordinal].is_some() {
                continue;
            }
            if let Some((_, from)) = hinted.iter().find(|(built, _)| *built == plan) {
                let borrowed = references[*from].clone();
                references[*ordinal] = borrowed;
            }
        }
    }

    /// The refusal a column the correlated re-walk never unified takes.
    fn pending_set_op_column() -> Error {
        Error::NotImplemented(
            "a set operation column naming an enclosing query's column that is not typed by the time the statement is planned is not supported".into(),
        )
    }

    /// One parse of a statement — one of [`Self::parse_statement`]'s two passes, and the
    /// whole of it for the statements that build no pending set-op column.
    fn parse_statement_once(&mut self) -> Result<Plan> {
        self.touched_tables.clear();
        self.cte_tables.clear();
        if self.ts.at_kw(Keyword::With) {
            self.parse_with_clause()?;
            let mark = self.touched_tables.len();
            let result = self
                .parse_with_statement_body()
                .and_then(|body| self.wrap_with_ctes(body, mark));
            self.ctes.clear();
            self.next_cte_rank = 0;
            self.cte_tables.clear();
            self.touched_tables.clear();
            // Belt-and-braces only. The real discharge is inside `parse_with_clause`,
            // which fires whichever verdict wins and leaves both slots empty on the one
            // path that reaches this line — and these two assignments sit *after* the
            // `parse_with_clause()?` above, so every error path skips them anyway. They
            // are here so a slot left set by a future early return cannot leak into the
            // next statement, which matters only if a caller ever recovers from a parse
            // error; none does today.
            self.with_clause_verdict = None;
            self.with_clause_arity_verdict = None;
            self.materialized_ctes.clear();
            self.defining_cte = None;
            self.pending_ctes.clear();
            self.recursive_ref_count = 0;
            self.with_recursive = false;
            return result;
        }
        self.parse_statement_body()
    }

    /// Wrap a parsed statement body in the [`Plan::With`] that materializes its `WITH`
    /// clause's data-modifying and recursive CTEs, in the order they were resolved — or
    /// return the bare body when there are none (read-only CTEs already inlined at their
    /// reference sites). `mark` is where the body's part of
    /// [`Self::touched_tables`] begins.
    ///
    /// The executor runs the entries in that order and then the body, against one
    /// storage. 18.6 runs every part of the statement against the snapshot taken before
    /// it, runs a data-modifying CTE when its reader first pulls a row, and runs the rest
    /// once the statement's own query is done, in the reverse of the resolved order. The
    /// shapes below, where the two could write or read differently, are refused here
    /// rather than run; the plain-`WITH` shapes this leaves are `EDGES.md` rows 160 and
    /// 162.
    ///
    /// # Errors
    ///
    /// `0A000`, before anything runs:
    ///
    /// - for a body that writes — a data-modifying statement, a sequence function —
    ///   beside a data-modifying entry: the body's write runs after the entry's here and
    ///   ahead of an unreferenced one there, and its reads see the entry's writes here
    ///   and not there;
    /// - for a recursive CTE resolved **after** a data-modifying one, which could read
    ///   the rows the earlier write left — measured, a recursive CTE scanning the table a
    ///   sibling `INSERT` wrote answers the inserted row here and not there;
    /// - under `WITH RECURSIVE`, for more than one data-modifying entry, whose order can
    ///   differ from 18.6's — which of two writers of one row writes it, and the order
    ///   their rows take a shared sequence's values; and for a body naming a table a
    ///   data-modifying entry can write, itself or through a foreign key's referential
    ///   action ([`written_tables`]), which would read the rows that write left.
    fn wrap_with_ctes(&mut self, body: Plan, mark: usize) -> Result<Plan> {
        use crate::planner::CteBody;
        let ctes = std::mem::take(&mut self.materialized_ctes);
        let Some(write) = ctes
            .iter()
            .position(|c| matches!(c.body, CteBody::Modifying { .. }))
        else {
            return Ok(if ctes.is_empty() {
                body
            } else {
                Plan::With {
                    ctes,
                    body: Box::new(body),
                }
            });
        };
        let writer = &ctes[write].name;
        if !body.is_read_only() {
            return Err(Error::NotImplemented(format!(
                "a statement that writes, after data-modifying WITH query \"{writer}\""
            )));
        }
        if let Some(read) = ctes[write..]
            .iter()
            .find(|c| matches!(c.body, CteBody::Recursive { .. }))
        {
            return Err(Error::NotImplemented(format!(
                "recursive WITH query \"{}\" resolved after data-modifying WITH query \"{writer}\" \
                 (it could read that query's writes)",
                read.name
            )));
        }
        if self.with_recursive {
            let mut writers = ctes.iter().filter_map(|c| match &c.body {
                CteBody::Modifying { plan, .. } => Some(plan),
                CteBody::Recursive { .. } => None,
            });
            let written = writers
                .next()
                .map_or_else(Vec::new, |plan| written_tables(plan, self.schema));
            if writers.next().is_some() {
                return Err(Error::NotImplemented(
                    "more than one data-modifying query in a WITH RECURSIVE list".into(),
                ));
            }
            if let Some(table) = self.touched_tables[mark..]
                .iter()
                .find(|t| written.contains(t))
            {
                return Err(Error::NotImplemented(format!(
                    "a WITH RECURSIVE statement reading table \"{table}\", which data-modifying \
                     WITH query \"{writer}\" can write"
                )));
            }
        }
        Ok(Plan::With {
            ctes,
            body: Box::new(body),
        })
    }

    /// The statement after a `WITH` list, which 18.6's grammar gives one of `SELECT`
    /// (`VALUES`, `TABLE` and a parenthesized query among its spellings), `INSERT`,
    /// `UPDATE`, `DELETE` and `MERGE`; any other statement is a syntax error at its first
    /// token. pglet parses neither `TABLE` nor `MERGE` anywhere, so after a `WITH` list
    /// those two are syntax errors here as well.
    fn parse_with_statement_body(&mut self) -> Result<Plan> {
        let query = [
            Keyword::Select,
            Keyword::Values,
            Keyword::Insert,
            Keyword::Update,
            Keyword::Delete,
        ]
        .into_iter()
        .any(|k| self.ts.at_kw(k));
        if !query && !self.ts.at(&TokenKind::LParen) {
            return Err(self.ts.syntax_error_at_or_near());
        }
        self.parse_statement_body()
    }

    /// Dispatch the statement body (after any `WITH` clause): `SELECT`, `INSERT`,
    /// `UPDATE`, `DELETE`, `CREATE`, `ALTER TABLE`/`ALTER SEQUENCE`, `DROP`,
    /// `TRUNCATE`, the pg_dump-restore-tolerance no-ops (`GRANT`/`REVOKE`/`COMMENT`),
    /// a minimal `DO $$ … $$` block, or the transaction-control statements.
    fn parse_statement_body(&mut self) -> Result<Plan> {
        // `SELECT …` or a parenthesized select-with-parens (`(SELECT …) UNION …`) —
        // both route through the SELECT front-end. At statement start a leading `(`
        // can only begin a parenthesized select in the supported grammar.
        if self.ts.at_kw(Keyword::Select) || self.ts.at(&TokenKind::LParen) {
            self.parse_select()
        } else if self.ts.at_kw(Keyword::Insert) {
            self.parse_insert()
        } else if self.ts.at_kw(Keyword::Update) {
            self.parse_update()
        } else if self.ts.at_kw(Keyword::Delete) {
            self.parse_delete()
        } else if self.ts.at_kw(Keyword::Create) {
            self.parse_create()
        } else if self.ts.at_kw(Keyword::Alter) {
            self.parse_alter()
        } else if self.ts.at_kw(Keyword::Grant) {
            self.parse_noop(crate::planner::NoopCommand::Grant)
        } else if self.ts.at_kw(Keyword::Revoke) {
            self.parse_noop(crate::planner::NoopCommand::Revoke)
        } else if self.ts.at_kw(Keyword::Comment) {
            self.parse_noop(crate::planner::NoopCommand::Comment)
        } else if self.ts.at_kw(Keyword::Drop) {
            self.parse_drop()
        } else if self.ts.at_kw(Keyword::Truncate) {
            self.parse_truncate()
        } else if self.ts.at_kw(Keyword::Copy) {
            self.parse_copy()
        } else if self.ts.at_kw(Keyword::Do) {
            self.parse_do()
        } else if self.ts.at_kw(Keyword::Begin) || self.ts.at_kw(Keyword::Start) {
            self.parse_begin()
        } else if self.ts.at_kw(Keyword::Commit) {
            self.parse_commit()
        } else if self.ts.at_kw(Keyword::Rollback) {
            self.parse_rollback()
        } else if self.ts.at_kw(Keyword::Savepoint) {
            self.parse_savepoint()
        } else if self.ts.at_kw(Keyword::Release) {
            self.parse_release()
        } else if let TokenKind::Keyword(kw) = self.ts.peek_kind() {
            // A keyword that begins a real Postgres statement pglet doesn't
            // implement in the parser → `0A000` (recognized statement kind, not
            // built). SET/SHOW are normally intercepted before the parser
            // (`is_set_command`/`is_show_command`); top-level VALUES is unmodeled.
            // Any
            // *other* keyword at statement start is a misplaced clause word
            // (`FROM 1`, `WHERE …`), not a statement keyword — Postgres reports a
            // `42601` syntax error, so pglet does too (not a `0A000` conflation).
            if matches!(kw, Keyword::Set | Keyword::Show | Keyword::Values) {
                Err(Error::NotImplemented(
                    "this statement kind is not yet supported in the native parser".into(),
                ))
            } else {
                Err(self.ts.syntax_error_at_or_near())
            }
        } else {
            // Not a keyword at all — a bare identifier / number / operator at
            // statement start (`SELEC 1`, `xyz`, `123abc`): a typo or garbage, not
            // a statement kind. Postgres: `42601` syntax error at or near "<tok>".
            Err(self.ts.syntax_error_at_or_near())
        }
    }
}

/// The tables a data-modifying CTE's `plan` can write: its target, and, transitively, each
/// table a foreign key's referential action can write. A `CASCADE` or `SET NULL` writes
/// the referencing rows — `ON DELETE` when the referenced table is deleted from, `ON
/// UPDATE` when an update (an `INSERT … ON CONFLICT DO UPDATE`'s included) assigns one of
/// the referenced columns; a cascaded delete is a delete of the referencing table, and the
/// other three an update of its referencing columns. `NO ACTION` and `RESTRICT` write
/// nothing, and neither does a plain `INSERT`. A table is listed whether or not a row the
/// statement reaches references one it changes, and as 18.6 writes it: one behind a key an
/// `ON DELETE SET NULL` nulls is listed although pglet's executor leaves it unwritten
/// (`EDGES.md` row 163).
fn written_tables(plan: &Plan, schema: &Schema) -> Vec<String> {
    use crate::catalog::ReferentialAction::{Cascade, SetNull};
    use crate::planner::{OnConflictAction, OnConflictClause};
    // Each write: the table, and the columns it assigns — `None` for a delete.
    let mut writes: Vec<(&str, Option<Vec<&str>>)> = match plan {
        Plan::Insert {
            table, on_conflict, ..
        } => {
            let columns = match on_conflict {
                Some(OnConflictClause {
                    action: OnConflictAction::DoUpdate { assignments, .. },
                    ..
                }) => assignments.iter().map(|(c, _)| c.as_str()).collect(),
                _ => Vec::new(),
            };
            vec![(table.as_str(), Some(columns))]
        }
        Plan::Update {
            table, assignments, ..
        } => vec![(
            table.as_str(),
            Some(assignments.iter().map(|(c, _)| c.as_str()).collect()),
        )],
        Plan::Delete { table, .. } => vec![(table.as_str(), None)],
        _ => Vec::new(),
    };
    let mut i = 0;
    while let Some((parent, columns)) = writes.get(i).cloned() {
        for (child, _, fk) in schema.referencing_foreign_keys(parent) {
            let action = match &columns {
                None => fk.on_delete,
                Some(set) if fk.parent_columns.iter().any(|c| set.contains(&c.as_str())) => {
                    fk.on_update
                }
                Some(_) => continue,
            };
            let write = match action {
                Cascade if columns.is_none() => None,
                Cascade | SetNull => Some(fk.columns.iter().map(String::as_str).collect()),
                _ => continue,
            };
            let write = (child.name.as_str(), write);
            if !writes.contains(&write) {
                writes.push(write);
            }
        }
        i += 1;
    }
    writes
        .into_iter()
        .map(|(table, _)| table.to_string())
        .collect()
}

#[cfg(test)]
mod statement_dispatch_tests {
    use crate::catalog::Schema;
    use crate::parser::parse;

    fn sqlstate(sql: &str) -> String {
        let schema = Schema::new();
        parse(sql, &schema)
            .expect_err("statement must fail to parse")
            .sqlstate()
            .to_string()
    }

    #[test]
    fn garbage_leading_token_is_42601_not_0a000() {
        // A typo, bare identifier, or a misplaced clause keyword at statement start
        // is a Postgres `42601` syntax error at that token — not the `0A000`
        // "unsupported construct" pglet used to conflate every unhandled leading
        // token with.
        for sql in [
            "SELEC 1",
            "FOOBAR 1",
            "xyz",
            "CREAT TABLE x(a int)",
            "FROM 1",
            "WHERE x = 1",
        ] {
            assert_eq!(
                sqlstate(sql),
                "42601",
                "{sql:?} should be a 42601 syntax error"
            );
        }
    }

    #[test]
    fn recognized_unimplemented_statement_stays_0a000() {
        // A keyword that begins a real Postgres statement pglet deliberately doesn't
        // implement in the parser stays `0A000` (recognized kind, not built) rather
        // than being reclassified as a syntax error.
        // `SAVEPOINT`/`RELEASE` left this list when savepoints landed, leaving
        // top-level `VALUES` as its only member — a keyword whose statement the
        // parser *does* build has no business here, and reading `0A000` off a stale
        // list would say the feature is absent while it works.
        assert_eq!(sqlstate("VALUES (1)"), "0A000");
    }
}

#[cfg(test)]
mod reservation_tests {
    use super::is_reserved;
    use crate::lexer::{ALL_KEYWORDS, Keyword, PgClass};

    #[test]
    fn reservation_tracks_the_colid_rule() {
        // The whole-set invariant: a keyword is reserved here iff it is *not* a
        // valid Postgres `ColId` (i.e. not UNRESERVED/COL_NAME). A future keyword
        // added with the wrong class, or a divergent `is_reserved` body, fails here.
        for &kw in ALL_KEYWORDS {
            let is_col_id = matches!(kw.pg_class(), PgClass::Unreserved | PgClass::ColName);
            assert_eq!(
                is_reserved(kw),
                !is_col_id,
                "{kw:?} ({:?}): reservation must equal !ColId",
                kw.pg_class(),
            );
        }
    }

    #[test]
    fn colid_keywords_are_not_reserved() {
        // The 22 over-reserved words Postgres accepts as bare column/table names.
        for kw in [
            Keyword::Insert,
            Keyword::Update,
            Keyword::Delete,
            Keyword::Set,
            Keyword::By,
            Keyword::Drop,
            Keyword::Alter,
            Keyword::Index,
            Keyword::Truncate,
            Keyword::Nulls,
            Keyword::First,
            Keyword::Last,
            Keyword::Conflict,
            Keyword::Nothing,
            Keyword::Begin,
            Keyword::Commit,
            Keyword::Rollback,
            Keyword::Start,
            Keyword::Transaction,
            Keyword::Recursive,
            Keyword::Values,  // COL_NAME
            Keyword::Between, // COL_NAME
        ] {
            assert!(
                !is_reserved(kw),
                "{kw:?} is a valid ColId, must not be reserved"
            );
        }
    }

    #[test]
    fn non_colid_keywords_stay_reserved() {
        // RESERVED and TYPE_FUNC_NAME words — Postgres rejects both as bare
        // columns, so pglet reserving them is correct conformance.
        for kw in [
            // RESERVED
            Keyword::Select,
            Keyword::From,
            Keyword::Where,
            Keyword::In,
            Keyword::Lateral,
            // TYPE_FUNC_NAME (`SELECT is FROM t` is a PG syntax error too)
            Keyword::Is,
            Keyword::Like,
            Keyword::Ilike,
            Keyword::Join,
            Keyword::Inner,
            Keyword::Left,
            Keyword::Right,
            Keyword::Full,
            Keyword::Outer,
            Keyword::Cross,
            Keyword::Natural,
        ] {
            assert!(is_reserved(kw), "{kw:?} is not a ColId, must stay reserved");
        }
    }
}

#[cfg(test)]
mod name_literal_tests {
    use super::reduce_name_literal;

    /// The name a literal reduces to, or its SQLSTATE.
    fn read(literal: &str) -> Result<String, String> {
        reduce_name_literal(literal).map_err(|e| e.sqlstate().to_string())
    }

    #[test]
    fn a_quoted_schema_part_is_never_folded() {
        // A quoted part is exact, so `"PUBLIC"` names no schema on either engine
        // (`42P01`, measured on 18.6 for the `nextval`, the `setval` and the
        // `DEFAULT nextval` spellings alike).
        assert_eq!(read(r#""PUBLIC".s"#), Err("42P01".to_string()));
        assert_eq!(read(r#""Public".s"#), Err("42P01".to_string()));
        // ...and the exact-match half of the same rule: a quoted part that *is*
        // `public` names the implicit schema, as it does on 18.6.
        assert_eq!(read(r#""public".s"#), Ok("s".to_string()));
    }

    #[test]
    fn an_unquoted_schema_part_is_downcased() {
        for literal in ["public.s", "PUBLIC.s", "PuBlIc.s"] {
            assert_eq!(read(literal), Ok("s".to_string()), "{literal}");
        }
    }

    #[test]
    fn the_relation_part_folds_only_when_unquoted() {
        // The two name positions must name the same relation: `CREATE SEQUENCE MySeq`
        // registers `myseq` (the identifier fold), so an unquoted literal has to reach
        // the same key, and a quoted one has to stay exact.
        assert_eq!(read("MySeq"), Ok("myseq".to_string()));
        assert_eq!(read("MYSEQ"), Ok("myseq".to_string()));
        assert_eq!(read(r#""MySeq""#), Ok("MySeq".to_string()));
        assert_eq!(read("public.MySeq"), Ok("myseq".to_string()));
        assert_eq!(read(r#"PUBLIC."MySeq""#), Ok("MySeq".to_string()));
    }

    #[test]
    fn a_non_public_schema_stays_loud_in_both_spellings() {
        assert_eq!(read("noschema.s"), Err("42P01".to_string()));
        assert_eq!(read(r#""noschema".s"#), Err("42P01".to_string()));
        assert_eq!(read("a.b.c"), Err("42601".to_string()));
    }
}
