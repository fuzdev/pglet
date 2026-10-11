//! DDL + transaction statement parsing, fused into the planner [`Plan`] IR.
//!
//! `CREATE TABLE`/`INDEX`/`TYPE`, `DROP`, `ALTER TABLE`, `TRUNCATE`, and the
//! transaction-control statements (`BEGIN`/`START TRANSACTION`/`COMMIT`/`ROLLBACK`).
//! The native parser builds the planner `Plan::Create*`/`Drop*`/`Truncate`/… nodes
//! directly — the lowering targets are catalog/planner types, so there is no
//! intermediate scaffold. The differential fixtures guard the plans against real
//! Postgres.

use crate::catalog::{ColumnDefault, IdentityGeneration, IndexKey, ReferentialAction};
use crate::executor::{IndexMethod, IsolationLevel};
use crate::lexer::{Keyword, TokenKind};
use crate::planner::{
    AddConstraint, BuiltinFn, CheckConstraintDef, ColumnDef, CopyDirection, CopyFormat,
    CopyOptions, Expr, ForeignKeyDef, Literal, NoopCommand, Plan, SequenceFunc, SequenceOwnerSpec,
    SequenceRestart, UniqueConstraint, UnperformedAlter, contains_aggregate, contains_window,
    expr_contains_subquery, is_serial_type,
};
use crate::types::Value;
use crate::{Error, Result};

use super::Parser;

/// The sequence options `CREATE`/`ALTER SEQUENCE` share, as written.
///
/// Every field is an `Option` so an *absent* clause stays distinguishable from one
/// written with the default value — `ALTER SEQUENCE s OWNER TO x` must leave the
/// start and the owner alone, not reset them.
#[derive(Default)]
struct SequenceOptions {
    /// `START [WITH] n`.
    start: Option<i64>,
    /// `RESTART` / `RESTART [WITH] n`.
    restart: Option<SequenceRestart>,
    /// `OWNED BY {table.column | NONE}`.
    owned_by: Option<SequenceOwnerSpec>,
}

/// The identity-column sequence options pglet models, as written.
///
/// The same "every collected field must be read" rule [`SequenceOptions`] carries:
/// this list is parsed at three sites (`CREATE TABLE`'s inline `GENERATED …`, `ADD
/// COLUMN`'s, and `ALTER COLUMN … ADD GENERATED`) and a field one of them drops is a
/// user-declared fact the engine then invents for itself.
#[derive(Default)]
struct IdentityOptions {
    /// `START [WITH] n` — the sequence's seed (`None` = `1`).
    start: Option<i64>,
    /// `SEQUENCE NAME <n>` — the **declared** name of the implicit sequence
    /// (`None` = the engine mints `<table>_<column>_seq`, disambiguated).
    sequence_name: Option<String>,
}

/// The object kind a `DROP` targets (the subset pglet supports).
enum DropKind {
    Table,
    Index,
    Type,
    Sequence,
}

/// A DDL context whose `Expr` Postgres restricts.
///
/// It carries the two message phrasings each one uses: Postgres spells the same
/// context singular in the subquery / column-reference messages and plural in the
/// window / aggregate / set-returning ones, so both are needed per context.
///
/// **All four are postcard-persisted into the catalog**, which is why their
/// restrictions are load-bearing rather than merely conformance — a column `DEFAULT`
/// joined the other three when `ColumnDefault` gained its `Expr` catch-all. The
/// three `CHECK`/index contexts are refused here, by
/// [`reject_persisted_expr_constructs`], which ranks its three kinds;
/// [`Self::Default`] is refused in `executor::ddl`'s `check_default_expr_nodes`,
/// which orders them *positionally* and runs at the statement position 18.6 cooks a
/// default. The two do not share a rule set — a column reference is legal in a
/// `CHECK` and forbidden in a `DEFAULT` — which is why the enum is shared and the
/// walks are not.
#[derive(Debug, Clone, Copy)]
pub enum PersistedExprContext {
    /// A CHECK constraint (column-level, table-level, or `ALTER TABLE ADD CONSTRAINT`).
    Check,
    /// A partial-index predicate (`CREATE INDEX … WHERE …`).
    IndexPredicate,
    /// An index key expression (`CREATE INDEX … ((expr))`).
    IndexExpression,
    /// A column `DEFAULT` expression — every route (`CREATE TABLE`, `ALTER TABLE …
    /// ADD COLUMN`, `ALTER TABLE … ALTER COLUMN … SET DEFAULT`), which Postgres
    /// analyzes under one `EXPR_KIND_COLUMN_DEFAULT` and so phrases identically.
    Default,
}

impl PersistedExprContext {
    /// The **plural** phrase Postgres uses for the window / aggregate /
    /// set-returning messages (`… are not allowed in check constraints`).
    pub const fn plural(self) -> &'static str {
        match self {
            Self::Check => "check constraints",
            Self::IndexPredicate => "index predicates",
            Self::IndexExpression => "index expressions",
            Self::Default => "DEFAULT expressions",
        }
    }

    /// The **singular** phrase Postgres uses for the subquery and column-reference
    /// messages (`cannot use subquery in check constraint`).
    pub const fn singular(self) -> &'static str {
        match self {
            Self::Check => "check constraint",
            Self::IndexPredicate => "index predicate",
            Self::IndexExpression => "index expression",
            Self::Default => "DEFAULT expression",
        }
    }
}

/// Reject the constructs Postgres forbids in a **persisted** `Expr` context — a
/// subquery (`0A000`), a window function (`42P20`), or an aggregate (`42803`) — at
/// DDL time, matching Postgres's exact SQLSTATEs and messages.
///
/// The order matches Postgres: a **subquery is checked first**, so a window (or
/// aggregate) reachable only *inside* a subquery yields `0A000` (`cannot use
/// subquery in …`) — Postgres never descends far enough to see the inner window —
/// rather than `42P20`. This ordering is what makes the persistence invariant
/// airtight: a CHECK / partial-index-predicate / index-expression `Expr` is
/// postcard-serialized into the catalog, and the only ways an [`Expr::Window`] (a
/// variant whose `order_by` field's serialized shape may evolve) — or any
/// plan-bearing `Expr` — could reach one are *directly* (caught by
/// [`contains_window`], which walks the scalar tree) or *nested in a subquery*
/// (caught here by [`expr_contains_subquery`], since [`contains_window`] treats a
/// subquery as an opaque leaf). Rejecting both closes every path, so no persisted
/// `Expr` ever contains a window/plan node and its shape can change without a
/// store-magic bump.
///
/// For the plan-bearing half this is the only gate raising Postgres's own error, but
/// not the only one standing before the catalog write: `executor::ddl` also probes the
/// tree by **evaluating** it — `validate_stored_expr` against a representative row here,
/// `cook_column_default` by resolving the default itself for a column `DEFAULT` — and
/// the evaluator has no execution context for a sub-plan, so each refuses independently
/// (`0A000 … subquery requires execution context`) — measured, by deleting each gate in
/// turn. Both must be gone before a
/// plan-bearing `Expr` is written; if one ever is, the store no longer opens, since a
/// [`Plan`] does not decode.
///
/// **Three whole-tree passes, and that is a conformance gap of its own.** Postgres
/// decides these by position rather than by kind — one bottom-up, left-to-right
/// walk — so an arrangement putting a *later*-ranked construct positionally first
/// answers differently here: measured on 18.6,
/// `CHECK (count(*) > 0 AND (SELECT true))` is `42803` while
/// `CHECK ((SELECT true) AND count(*) > 0)` is `0A000`, where both are `0A000` here
/// because the subquery pass runs first. `executor::ddl`'s `check_default_expr_nodes` is the
/// positional shape; converting these three contexts to it is separate work
/// (recorded in `EDGES.md`).
///
/// **Invariant: [`PersistedExprContext::Default`] must never be passed here.** The
/// enum names four contexts because it owns the message phrasings, but only three
/// of them take *this* walk — a column `DEFAULT` has a positional walk of its own,
/// and routing it through the ranked one would silently regress the very rule
/// `executor::ddl`'s `check_default_expr_nodes` exists to hold, answering the *highest-ranked*
/// construct in the tree rather than the positionally first. Nothing enforces it
/// mechanically — an assert would be dead code as long as no caller does it — and
/// the enum is deliberately not split, because converting the other three contexts
/// to the positional walk merges the two sets again.
fn reject_persisted_expr_constructs(expr: &Expr, ctx: PersistedExprContext) -> Result<()> {
    if expr_contains_subquery(expr) {
        return Err(Error::SubqueryNotAllowed {
            context: ctx.singular(),
        });
    }
    if contains_window(expr) {
        return Err(Error::WindowingError {
            context: ctx.plural(),
        });
    }
    if contains_aggregate(expr) {
        return Err(Error::AggregateNotAllowed {
            context: ctx.plural(),
        });
    }
    Ok(())
}

/// 18.6's `42601` for a column carrying both a `DEFAULT` and a `GENERATED … AS
/// IDENTITY` clause.
///
/// **One sentence, two raisers, in one written order each** — the `DEFAULT` clause
/// tests `saw_identity` and the identity clause tests `saw_default` — so the two
/// spellings must not be able to answer differently. A fixture cannot separate them:
/// both are `42601`, which is also the code of every other refusal on this statement.
fn default_and_identity_conflict(column: &str, table: &str) -> Error {
    Error::syntax(format!(
        "both default and identity specified for column \"{column}\" of table \"{table}\""
    ))
}

/// Normalize an [`Expr`] that *is* one of the closed [`ColumnDefault`] forms back
/// onto that form, or `None` when it is a general expression.
///
/// The inverse of [`column_default_to_expr`], and the reason the parenthesized
/// spellings work: 18.6 strips redundant parens in parse analysis, so `DEFAULT (5)`,
/// `DEFAULT (-7)`, `DEFAULT (now())` and `DEFAULT (nextval('s'))` are each identical
/// to their bare form there — measured, including that the parenthesized `nextval`
/// records the same `pg_attrdef` dependency edge. pglet's parser drops the parens
/// too (they are structural, not a wrapper node), so the tree that arrives here is
/// exactly the bare form's.
///
/// Deliberately **narrow**: only a bare literal (sign already folded by `negate`, or
/// carried as an [`Expr::UnaryPlus`] wrapper the `+` form leaves behind) and the
/// recognized niladic/sequence functions. Everything it declines — a cast, an
/// arithmetic node, a `nextval` with extra arguments, and `LOCALTIMESTAMP` /
/// `LOCALTIME`, which are [`Expr::Builtin`] too — lands on [`ColumnDefault::Expr`]
/// and is stored unevaluated. The narrowness is what stops those last two from
/// rendering back as `now()` / `CURRENT_TIME`, which is the wrong spelling for an
/// introspecting client and for the dump.
fn column_default_from_expr(expr: &Expr) -> Option<ColumnDefault> {
    match expr {
        Expr::Literal(lit) => Some(ColumnDefault::Literal(literal_to_default_value(lit))),
        // `+1.5` keeps its operator node (Postgres's `doNegate` folds only `-`), so
        // the recognized bare-signed-literal form has to be seen through it.
        Expr::UnaryPlus(inner) => match inner.as_ref() {
            Expr::Literal(lit @ (Literal::Int(_) | Literal::Float(_) | Literal::Numeric(_))) => {
                Some(ColumnDefault::Literal(literal_to_default_value(lit)))
            }
            _ => None,
        },
        Expr::Builtin(BuiltinFn::Now) => Some(ColumnDefault::Now),
        Expr::Builtin(BuiltinFn::TransactionTimestamp) => Some(ColumnDefault::TransactionTimestamp),
        Expr::Builtin(BuiltinFn::StatementTimestamp) => Some(ColumnDefault::StatementTimestamp),
        Expr::Builtin(BuiltinFn::ClockTimestamp) => Some(ColumnDefault::ClockTimestamp),
        Expr::Builtin(BuiltinFn::CurrentTimestamp) => Some(ColumnDefault::CurrentTimestamp),
        Expr::Builtin(BuiltinFn::CurrentDate) => Some(ColumnDefault::CurrentDate),
        Expr::Builtin(BuiltinFn::CurrentTime) => Some(ColumnDefault::CurrentTime),
        Expr::Builtin(BuiltinFn::RandomUuid) => Some(ColumnDefault::RandomUuid),
        Expr::SequenceFn {
            func: SequenceFunc::Nextval,
            seq,
            args,
        } if args.is_empty() => Some(ColumnDefault::NextVal(seq.clone())),
        _ => None,
    }
}

/// The [`Value`] a literal `DEFAULT` stores — the lexer's [`Literal`] carried across
/// unchanged, one variant to one variant.
///
/// [`Literal::Numeric`] maps to [`Value::Numeric`], not `Value::Float8`. It used to
/// go through `f64` because a token-level classifier ahead of this function parsed a
/// non-`i64` number with `f64::from_str` and this mapping was written to reproduce
/// *that*; the classifier's literal arms are gone (see
/// [`Parser::try_recognized_default`]), and the `f64` hop was lossy in the direction
/// Postgres is exact — `DEFAULT 1.50` is a `numeric` constant of scale 2 there
/// (rendered `1.50` by `pg_get_expr`), and `1e30` is exact rather than the nearest
/// double.
fn literal_to_default_value(lit: &Literal) -> Value {
    match lit {
        Literal::Null => Value::Null,
        Literal::Bool(b) => Value::Bool(*b),
        Literal::Int(i) => Value::Int8(*i),
        Literal::Float(f) => Value::Float8(*f),
        Literal::Numeric(n) => Value::Numeric(n.clone()),
        Literal::String(s) => Value::Text(s.clone()),
    }
}

/// The [`ColumnDefault`] `expr` becomes: its closed form when it *is* one, else the
/// [`ColumnDefault::Expr`] catch-all.
///
/// **Total, and deliberately so.** The refusals Postgres raises for a column
/// `DEFAULT` — a subquery, an aggregate, a window function, a set-returning
/// function, a column reference, a `$N`, a bare `DEFAULT` — are *not* raised here:
/// they are parse-analysis errors on 18.6, and pglet's parser runs ahead of every
/// statement-level gate they must lose to. They run in the executor instead, at the
/// position Postgres cooks defaults — `executor::ddl`'s `cook_column_default`, whose
/// own doc carries the measured members and the route split (a member of one route's
/// list is not a member of the other's by inheritance) — which is what the whole
/// `Expr` arm made possible: before it there was nothing to hand the executor but a
/// refusal.
fn default_from_expr(expr: &Expr) -> ColumnDefault {
    column_default_from_expr(expr).unwrap_or_else(|| ColumnDefault::Expr(expr.clone()))
}

/// Reduce a `DEFAULT NULL` to **no default**, which is what 18.6 stores.
///
/// `AddRelationNewConstraints` skips `StoreAttrDefault` when the cooked expression is
/// a NULL `Const`, so a `DEFAULT NULL` column carries **zero** `pg_attrdef` rows —
/// measured on 18.6: no row, `information_schema`'s `column_default` SQL NULL, and
/// `pg_dump` emitting no `DEFAULT` clause. On the `ALTER COLUMN … SET DEFAULT NULL`
/// route it *removes* an existing row (measured: 1 → 0), so that spelling is
/// `DROP DEFAULT`. Both routes reach this function, and both already mean "no default"
/// downstream: the column-definition caller stores `Column.default = None`, and
/// `Plan::AlterTableSetDefault { default: None }` is exactly what `DROP DEFAULT` builds.
///
/// Two live wrongs came from keeping the shape instead, and they differ in severity.
/// `ALTER TABLE … ADD COLUMN c INT NOT NULL DEFAULT NULL` over a non-empty table
/// wrote NULLs into a `NOT NULL` column (18.6: `23502`), after which an index built
/// over that column made an accelerated-profile `ORDER BY` read **(0 rows)**. And
/// `information_schema.columns.column_default` reported the four characters `NULL`
/// where 18.6 reports SQL NULL, so a migration tool testing `column_default IS NOT
/// NULL` saw a default that does not exist — on the `SET DEFAULT NULL` route too, which
/// is the only way *that* route was observably wrong: it did clear the old value, but
/// stored a NULL-valued default rather than none, and the two resolve identically.
///
/// **The test is 18.6's own — "is the cooked expression a NULL `Const`" — and one
/// level of cast is what that answers yes to.** `coerce_type` folds an `unknown`
/// literal into a typed `Const` in place, so `NULL::int`, `(NULL)::int` and
/// `CAST(NULL AS int)` are each still a NULL `Const` at the end of parse analysis
/// and store **no** `pg_attrdef` row; a *second* cast is a real conversion
/// (`CoerceViaIO`) rather than a constant, so `NULL::text::int` **is** stored, and
/// `pg_get_expr` reads it back as `(NULL::text)::integer`. Measured on 18.6, all
/// five spellings plus `1 + NULL` (stored, `(1 + NULL::integer)`) — which is the
/// control that identifies the rule as "a constant" rather than "contains a NULL".
///
/// So the reduction looks through exactly one [`Expr::Cast`] / [`Expr::CastTypmod`]
/// over a bare `NULL` literal. Deeper nesting is left alone rather than folded, in
/// the direction that keeps a default rather than dropping one.
fn without_null_default(default: ColumnDefault) -> Option<ColumnDefault> {
    match &default {
        ColumnDefault::Literal(Value::Null) => None,
        ColumnDefault::Expr(Expr::Cast { expr, .. } | Expr::CastTypmod { expr, .. })
            if matches!(expr.as_ref(), Expr::Literal(Literal::Null)) =>
        {
            None
        }
        _ => Some(default),
    }
}

/// The [`Expr`] a recognized [`ColumnDefault`] would have parsed to — the inverse of
/// [`column_default_from_expr`].
///
/// Used only to **seed** the operator loop when the token-level classifier has
/// already consumed a recognized form and more of an expression follows
/// (`DEFAULT 5 + a`). The tree is then discarded: either the loop consumed nothing
/// and the classifier's own `ColumnDefault` is returned, or it consumed something and
/// the whole expression is normalized afresh. It is built in full rather than as a
/// placeholder anyway, because the extended tree is what
/// `executor::ddl`'s `check_default_expr_nodes` later reads, and a placeholder would
/// be a verdict about a node the source never wrote.
fn column_default_to_expr(default: &ColumnDefault) -> Expr {
    match default {
        ColumnDefault::Expr(expr) => expr.clone(),
        ColumnDefault::Literal(value) => match value {
            Value::Null => Expr::Literal(Literal::Null),
            Value::Bool(b) => Expr::Literal(Literal::Bool(*b)),
            Value::Int8(i) => Expr::Literal(Literal::Int(*i)),
            Value::Float8(f) => Expr::Literal(Literal::Float(*f)),
            Value::Numeric(n) => Expr::Literal(Literal::Numeric(n.clone())),
            Value::Text(s) => Expr::Literal(Literal::String(s.clone())),
            // The classifier produces only the five spellings above. Any other
            // `Value` seeds an `Expr::Bound` — "a constant of this value", which is
            // true of it and invents no source spelling — rather than a plausible
            // wrong `Literal`. Either way it is a leaf to the walk that reads it.
            other => Expr::Bound(other.clone()),
        },
        ColumnDefault::Now => Expr::Builtin(BuiltinFn::Now),
        ColumnDefault::TransactionTimestamp => Expr::Builtin(BuiltinFn::TransactionTimestamp),
        ColumnDefault::StatementTimestamp => Expr::Builtin(BuiltinFn::StatementTimestamp),
        ColumnDefault::ClockTimestamp => Expr::Builtin(BuiltinFn::ClockTimestamp),
        ColumnDefault::CurrentTimestamp => Expr::Builtin(BuiltinFn::CurrentTimestamp),
        ColumnDefault::CurrentDate => Expr::Builtin(BuiltinFn::CurrentDate),
        ColumnDefault::CurrentTime => Expr::Builtin(BuiltinFn::CurrentTime),
        ColumnDefault::RandomUuid => Expr::Builtin(BuiltinFn::RandomUuid),
        ColumnDefault::NextVal(seq) => Expr::SequenceFn {
            func: SequenceFunc::Nextval,
            seq: seq.clone(),
            args: Vec::new(),
        },
    }
}

/// A parsed column definition plus its column-level constraints. The caller folds
/// each constraint into the table-scoped lists; [`ParsedColumn::nullable`] is the
/// declaration both callers take.
struct ParsedColumn {
    name: String,
    type_name: String,
    /// A `NOT NULL` column constraint was present.
    not_null: bool,
    /// A column-level `PRIMARY KEY` was present (also forces NOT NULL).
    has_primary: bool,
    /// Column-level `PRIMARY KEY` / `UNIQUE` constraints, in declared order (each a
    /// single-column `UniqueConstraint`).
    column_uniques: Vec<UniqueConstraint>,
    /// The resolved `DEFAULT`, `None` for none — and always `None` when
    /// `implicit_sequence` is set (the engine's sequence name is minted at execute
    /// time, not here).
    default: Option<ColumnDefault>,
    /// `SERIAL` or `GENERATED … AS IDENTITY` — the column takes an implicit
    /// `nextval` default over an engine-named sequence, and is implicitly `NOT NULL`
    /// ([`ParsedColumn::nullable`]).
    implicit_sequence: bool,
    /// Column-level `CHECK` constraints, each carrying its declared `CONSTRAINT <n>`
    /// name when one was given.
    checks: Vec<CheckConstraintDef>,
    /// A single inline `REFERENCES` foreign key, if captured (still carrying its
    /// declared name, or `None` for the executor to mint).
    foreign_key: Option<ForeignKeyDef>,
    /// The identity generation if `GENERATED … AS IDENTITY` was present, else
    /// `None`. Feeds `implicit_sequence` (an identity column is a SERIAL plus the
    /// `428C9` rule) and the catalog `Column.identity`.
    identity: Option<IdentityGeneration>,
    /// The identity's `START WITH` seed, if given (`None` = default `1`).
    identity_start: Option<i64>,
    /// The identity's declared `SEQUENCE NAME <n>`, if given — the *user's* name for
    /// the implicit sequence, taken verbatim (`None` = the engine mints one).
    identity_sequence_name: Option<String>,
}

impl ParsedColumn {
    /// Whether the column is declared nullable: it is not where it says `NOT NULL`, is a
    /// column-level `PRIMARY KEY`, or is a `SERIAL` or an identity column, each of which
    /// 18.6 declares `NOT NULL` too.
    ///
    /// One derivation for `CREATE TABLE` and `ALTER TABLE … ADD COLUMN`, so a column
    /// written the same way is declared the same way by both. `ADD COLUMN`'s back-fill
    /// is what holds the declaration over rows that already exist: the executor refuses
    /// a NULL back-fill value under it (`23502`) ahead of the catalog write.
    const fn nullable(&self) -> bool {
        !self.implicit_sequence && !self.not_null && !self.has_primary
    }
}

/// A parsed table-level constraint, before it folds into the table-scoped lists.
enum ParsedTableConstraint {
    PrimaryKey {
        name: Option<String>,
        columns: Vec<String>,
    },
    Unique {
        name: Option<String>,
        columns: Vec<String>,
    },
    Check(CheckConstraintDef),
    ForeignKey(ForeignKeyDef),
}

/// One element of a `CREATE TABLE (…)` list, kept in **written** order.
///
/// Postgres walks `stmt->tableElts` once (`transformCreateStmt`), routing a
/// `T_ColumnDef` to `transformColumnDefinition` and a `T_Constraint` to
/// `transformTableConstraint` — both of which push onto the *same*
/// `cxt.ixconstraints` / `cxt.ckconstraints` / `cxt.fkconstraints` lists. So a
/// table-level constraint written *before* a column's inline one is resolved, and
/// therefore **named**, first. Routing the two kinds to separate sinks and
/// concatenating them reverses that for the mixed arrangement, which is observable
/// once the minted name is persisted: `CREATE TABLE ro (FOREIGN KEY (a) REFERENCES
/// p (id) ON DELETE CASCADE, a INT REFERENCES p (id) ON DELETE SET NULL)` pairs
/// `ro_a_fkey` with CASCADE on Postgres (measured on 18.6) and with SET NULL under a
/// two-sink fold — the same name on the wrong specification.
enum TableElement {
    Column(Box<ParsedColumn>),
    Constraint(ParsedTableConstraint),
}

impl Parser<'_> {
    /// Dispatch a `CREATE …` statement (`TABLE` / `[UNIQUE] INDEX` / `TYPE`).
    pub(super) fn parse_create(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Create)?;
        if self.ts.at_kw(Keyword::Table) {
            self.parse_create_table()
        } else if self.ts.at_kw(Keyword::Unique) || self.ts.at_kw(Keyword::Index) {
            self.parse_create_index()
        } else if self.ts.at_kw(Keyword::Sequence) {
            self.parse_create_sequence()
        } else if self.at_ident("type") {
            self.parse_create_type()
        } else if self.at_ident("schema") {
            self.parse_create_schema()
        } else if self.at_ident("extension") {
            self.parse_create_extension()
        } else {
            Err(Error::NotImplemented(
                "only CREATE TABLE / INDEX / SEQUENCE / TYPE / SCHEMA / EXTENSION are supported"
                    .into(),
            ))
        }
    }

    /// Parse `CREATE SEQUENCE [IF NOT EXISTS] name [options]` into a
    /// [`Plan::CreateSequence`] (pg_dump emits one per SERIAL column).
    ///
    /// `START WITH n`, `RESTART [[WITH] n]` and `OWNED BY {table.col | NONE}` are
    /// captured; the options that only affect a step size pglet doesn't model — `AS
    /// <type>`, `INCREMENT [BY] n`, `[NO] MINVALUE/MAXVALUE n`, `CACHE n`, `[NO]
    /// CYCLE` — are parsed and ignored. The option words are contextual identifiers
    /// (unreserved in Postgres), so they stay usable as ordinary names elsewhere.
    ///
    /// **Every field [`SequenceOptions`] collects is read here.** The struct is
    /// shared with `ALTER SEQUENCE`, and a field one caller collects and the other
    /// silently drops is a loud statement turned into a wrong value: `RESTART` was
    /// unconsumed (and so `42601`) before the two shared a parser, and dropping it
    /// here made `CREATE SEQUENCE s RESTART WITH 5` hand out `1`.
    fn parse_create_sequence(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Sequence)?;
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.parse_relation_name()?;
        let mut opts = SequenceOptions::default();
        self.parse_sequence_options(&mut opts)?;
        Ok(Plan::CreateSequence {
            name,
            start: opts.start.unwrap_or(1),
            restart: opts.restart,
            if_not_exists,
            owned_by: opts.owned_by,
        })
    }

    /// Parse `ALTER SEQUENCE [IF EXISTS] name [options]` into a
    /// [`Plan::AlterSequence`] (the leading `SEQUENCE` keyword is already consumed).
    ///
    /// The recognized option tail is shared with `CREATE SEQUENCE`
    /// ([`Self::parse_sequence_options`]) plus `RESTART [[WITH] n]`. Anything the
    /// option loop doesn't recognize — `OWNER TO`, `RENAME TO`, `SET SCHEMA`, the
    /// pg_dump preamble's spellings — is consumed to the statement boundary and has
    /// no effect, as it did when the whole statement was a [`Plan::Noop`]. The
    /// *target* is resolved either way, which is the one behavior change: an
    /// `ALTER SEQUENCE` naming nothing is `42P01` and one naming a table is `42809`,
    /// where both used to succeed silently.
    fn parse_alter_sequence(&mut self) -> Result<Plan> {
        let if_exists = self.parse_if_exists()?;
        let name = self.parse_relation_name()?;
        let mut opts = SequenceOptions::default();
        self.parse_sequence_options(&mut opts)?;
        // Tolerate an unrecognized tail (see the doc comment).
        while !self.ts.at_eof() && !self.ts.at(&TokenKind::Semicolon) {
            self.ts.bump()?;
        }
        Ok(Plan::AlterSequence {
            name,
            if_exists,
            start: opts.start,
            restart: opts.restart,
            owned_by: opts.owned_by,
        })
    }

    /// Parse the shared `CREATE`/`ALTER SEQUENCE` option list into `opts`, stopping
    /// at the first token that isn't a sequence option.
    ///
    /// Options are space-separated (no commas). A later spelling of the same option
    /// overwrites an earlier one, which is `init_params`' behavior too.
    ///
    /// `AS <type>` discards the resolved type but still **reads** the name here, so
    /// refusals the *grammar* raises reach both statements while refusals that need the
    /// resolved type do not. Measured on both engines, at `CREATE` and `ALTER` alike:
    /// `AS FLOAT(0)` / `AS FLOAT(54)` are `22023`, and `AS timestamp(3,1)` /
    /// `AS interval(-1)` are `42601`. See [`Plan::CreateSequence`] /
    /// [`Plan::AlterSequence`] for what the discard costs.
    fn parse_sequence_options(&mut self, opts: &mut SequenceOptions) -> Result<()> {
        loop {
            if self.ts.eat_kw(Keyword::As)? {
                // AS <data type> — the name is read (so `opt_float`'s `22023` escapes)
                // and the resolved type discarded
                self.parse_type_name()?;
            } else if self.ts.eat_kw(Keyword::Start)? {
                self.ts.eat_kw(Keyword::With)?; // optional WITH
                opts.start = Some(self.parse_signed_int_literal()?);
            } else if self.eat_ident("restart")? {
                // `RESTART` (bare — back to START WITH) | `RESTART [WITH] n`. The
                // value is optional, so an explicit `WITH` is the only thing that
                // makes it mandatory; otherwise it is present iff a number follows.
                let with = self.ts.eat_kw(Keyword::With)?;
                opts.restart = Some(if with || self.at_int_literal() {
                    SequenceRestart::To(self.parse_signed_int_literal()?)
                } else {
                    SequenceRestart::ToStart
                });
            } else if self.eat_ident("increment")? {
                self.ts.eat_kw(Keyword::By)?; // optional BY
                self.parse_signed_int_literal()?; // ignored
            } else if self.ts.eat_kw(Keyword::No)? {
                // NO MINVALUE / NO MAXVALUE / NO CYCLE.
                if !(self.eat_ident("minvalue")?
                    || self.eat_ident("maxvalue")?
                    || self.eat_ident("cycle")?)
                {
                    return Err(self.ts.expected("MINVALUE, MAXVALUE, or CYCLE after NO"));
                }
            } else if self.eat_ident("minvalue")?
                || self.eat_ident("maxvalue")?
                || self.eat_ident("cache")?
            {
                self.parse_signed_int_literal()?; // ignored
            } else if self.eat_ident("cycle")? {
                // CYCLE — ignored.
            } else if self.eat_ident("owned")? {
                self.ts.expect_kw(Keyword::By)?;
                opts.owned_by = Some(self.parse_owned_by()?);
            } else {
                return Ok(());
            }
        }
    }

    /// Whether the current token starts an integer literal (a digit or a leading `-`).
    fn at_int_literal(&self) -> bool {
        matches!(self.ts.peek_kind(), TokenKind::Number) || self.ts.at(&TokenKind::Minus)
    }

    /// Parse `CREATE SCHEMA [IF NOT EXISTS] name` into a [`Plan::CreateSchema`].
    ///
    /// `SCHEMA` is a contextual identifier (not reserved), so it stays usable as
    /// an ordinary name. pglet models a single implicit `public` schema; the
    /// executor accepts `public` (a no-op) and fails loud on any other name. The
    /// optional `AUTHORIZATION role` clause is not supported.
    fn parse_create_schema(&mut self) -> Result<Plan> {
        self.expect_ident("schema")?;
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.parse_relation_name()?;
        Ok(Plan::CreateSchema {
            name,
            if_not_exists,
        })
    }

    /// Parse `CREATE [UNIQUE] INDEX [CONCURRENTLY] [[IF NOT EXISTS] name] ON table [USING method]
    /// ( key_part [, …] ) [NULLS [NOT] DISTINCT] [WHERE predicate]` into a
    /// [`Plan::CreateIndex`].
    ///
    /// A key part is a bare column (→ `IndexKey::Columns`, the position-keyed path)
    /// or an expression like `LOWER(email)` / `COALESCE(scope_kind, '')` (→
    /// `IndexKey::Exprs`); a mixed key folds to all-expressions. Per-part opclass
    /// (`text_pattern_ops`) and ordering (`ASC`/`DESC`/`NULLS FIRST|LAST`) are
    /// consumed and ignored — only the key expression matters for our equality-keyed
    /// indexes. The partial predicate parses to raw IR (no type-resolution).
    fn parse_create_index(&mut self) -> Result<Plan> {
        let unique = self.ts.eat_kw(Keyword::Unique)?;
        self.ts.expect_kw(Keyword::Index)?;
        // `CONCURRENTLY` sits between `INDEX` and `IF NOT EXISTS`
        // (`gram.y`'s `IndexStmt: CREATE opt_unique INDEX opt_concurrently
        // opt_single_name ON …`), and the order is enforced: 18.6 answers
        // `CREATE INDEX IF NOT EXISTS CONCURRENTLY i ON t (c)` with
        // `42601 syntax error at or near "CONCURRENTLY"`. It is a contextual
        // identifier, not a keyword, so a `"concurrently"`-quoted word is still an
        // index *name* — as it is on 18.6.
        let concurrently = self.eat_ident("concurrently")?;
        let if_not_exists = self.parse_if_not_exists()?;
        // The name is optional in the plain spelling only (`gram.y`'s `IndexStmt`:
        // `opt_single_name` in the first production, a required `name` behind
        // `IF NOT EXISTS`), and `ON` is reserved, so an `ON` here is the clause and
        // never a name: behind `IF NOT EXISTS` it is 18.6's `syntax error at or near
        // "ON"`. An absent name is the executor's to mint.
        let name = if self.ts.at_kw(Keyword::On) {
            if if_not_exists {
                return Err(self.ts.syntax_error_at_or_near());
            }
            None
        } else {
            Some(self.parse_relation_name()?)
        };
        self.ts.expect_kw(Keyword::On)?;
        let table = self.parse_relation_name()?;

        // `USING <method>` — an access-method *name* (`access_method_clause: USING
        // name`), so `expect_name` has already downcased an unquoted spelling and
        // kept a quoted one exact. `USING "GIN"` therefore names an access method no
        // engine has: 18.6 answers `42704 access method "GIN" does not exist`, pglet
        // the `0A000` it already gives every unrecognized method (`USING hash`,
        // `USING nosuchmethod`), so the spelling joins that set rather than being
        // silently accepted as `gin`.
        let method = if self.ts.eat_kw(Keyword::Using)? {
            let m = self.expect_name("an index method")?;
            match m.as_str() {
                "gin" => IndexMethod::Gin,
                "btree" => IndexMethod::Btree,
                other => {
                    return Err(Error::NotImplemented(format!(
                        "index method {other} is not supported (only btree and gin)"
                    )));
                }
            }
        } else {
            IndexMethod::Btree
        };

        // The key part list.
        self.ts.expect(&TokenKind::LParen)?;
        let parts = self.comma_list(Self::parse_index_key_part)?;
        self.ts.expect(&TokenKind::RParen)?;

        // All-bare-column → `Columns` (position-keyed); any expression part → `Exprs`
        // (the any-expression fold). A `match` (not `map_or`) so the `None` arm moves
        // `parts` lazily — `map_or`'s eager default would build (and discard)
        // `Exprs(parts)` even on the `Columns` path.
        #[allow(clippy::option_if_let_else)]
        let key = match bare_column_names(&parts) {
            Some(names) => IndexKey::Columns(names),
            None => IndexKey::Exprs(parts),
        };

        // `NULLS [NOT] DISTINCT` (index-level), then the partial `WHERE`.
        let mut nulls_not_distinct = false;
        if self.ts.eat_kw(Keyword::Nulls)? {
            nulls_not_distinct = self.ts.eat_kw(Keyword::Not)?;
            self.ts.expect_kw(Keyword::Distinct)?;
        }
        let predicate = if self.ts.eat_kw(Keyword::Where)? {
            let pred = self.parse_expr_bp(0)?;
            reject_persisted_expr_constructs(&pred, PersistedExprContext::IndexPredicate)?;
            Some(pred)
        } else {
            None
        };

        Ok(Plan::CreateIndex {
            name,
            table,
            key,
            unique,
            enforced: unique,
            predicate,
            nulls_not_distinct,
            method,
            if_not_exists,
            concurrently,
        })
    }

    /// Parse one index key part: an expression, with an optional trailing opclass
    /// identifier and `ASC`/`DESC` / `NULLS FIRST|LAST` ordering — all consumed and
    /// ignored.
    fn parse_index_key_part(&mut self) -> Result<Expr> {
        let expr = self.parse_expr_bp(0)?;
        reject_persisted_expr_constructs(&expr, PersistedExprContext::IndexExpression)?;
        // An opclass (`text_pattern_ops`) is a bare identifier following the key.
        if matches!(self.ts.peek_kind(), TokenKind::Identifier) {
            self.ts.bump()?;
        }
        let _ = self.ts.eat_kw(Keyword::Asc)? || self.ts.eat_kw(Keyword::Desc)?;
        if self.ts.eat_kw(Keyword::Nulls)? && !self.ts.eat_kw(Keyword::First)? {
            self.ts.expect_kw(Keyword::Last)?;
        }
        Ok(expr)
    }

    /// Parse `CREATE EXTENSION [IF NOT EXISTS] name [WITH] [SCHEMA s] [VERSION v]
    /// [CASCADE]` into a [`Plan::Noop`] tagged `CREATE EXTENSION`.
    ///
    /// **pglet models no extension catalog.** The two extensions a ported migration
    /// actually asks for are already *in* the engine unconditionally —
    /// `gen_random_uuid()` (pgcrypto) and `uuid_generate_v4()` (uuid-ossp) resolve
    /// with or without this statement — so there is no installation state to record
    /// and the statement has nothing to do. What is left is the one decision that is
    /// still knowable without state, and it is knowable exactly: whether the **name**
    /// is one this engine provides. An unknown name is 18.6's
    /// `0A000 extension "…" is not available`, which is that server's answer under
    /// `IF NOT EXISTS` too (measured).
    ///
    /// The residual, which a persisted set would close and is recorded rather than
    /// hidden: without installation state a **bare repeat** succeeds here where 18.6
    /// answers `42710 extension "pgcrypto" already exists`, and so does a bare
    /// `CREATE EXTENSION plpgsql`, which 18.6 refuses from the first statement
    /// because it ships pre-installed. The `IF NOT EXISTS` spelling — what every
    /// migration writes — agrees on both engines.
    ///
    /// The option tail is Postgres's `create_extension_opt_list`: the items may
    /// repeat in any order after an optional `WITH`, and each is parsed and dropped.
    fn parse_create_extension(&mut self) -> Result<Plan> {
        self.expect_ident("extension")?;
        let _if_not_exists = self.parse_if_not_exists()?;
        let name = self.expect_name("an extension name")?;
        require_known_extension(&name)?;
        self.ts.eat_kw(Keyword::With)?; // the optional noise word
        loop {
            if self.eat_ident("schema")? {
                // pglet models exactly one schema, so the name is checkable without
                // any extension state: 18.6 answers `CREATE EXTENSION pgcrypto
                // SCHEMA nosuchschema` with `3F000`, and so does this.
                let schema = self.expect_name("a schema name")?;
                if schema != "public" {
                    return Err(Error::SchemaNotFound(schema));
                }
            } else if self.eat_ident("version")? {
                // `VERSION` takes a `NonReservedWord_or_Sconst` — a bare word or a
                // string literal.
                if matches!(self.ts.peek_kind(), TokenKind::String) {
                    self.ts.bump()?;
                } else {
                    self.expect_name("an extension version")?;
                }
            } else if self.ts.eat_kw(Keyword::Cascade)? {
                // CASCADE — pglet installs no dependency, so nothing to cascade to.
            } else {
                break;
            }
        }
        Ok(Plan::Noop {
            tag: NoopCommand::CreateExtension,
        })
    }

    /// Parse `DROP EXTENSION [IF EXISTS] name [, …] [CASCADE | RESTRICT]` into a
    /// [`Plan::Noop`] tagged `DROP EXTENSION` (the leading `EXTENSION` word is
    /// already consumed).
    ///
    /// The mirror of [`Self::parse_create_extension`]'s model: with no installation
    /// state the only decision is the name. An unknown one is `42704` without
    /// `IF EXISTS` and a skip with it — both 18.6's answers — while a **known** name
    /// succeeds either way, which over-accepts a `DROP EXTENSION pgcrypto` that
    /// nothing created (18.6: `42704`).
    fn parse_drop_extension(&mut self) -> Result<Plan> {
        let if_exists = self.parse_if_exists()?;
        let names = self.comma_list(|p| p.expect_name("an extension name"))?;
        if !if_exists {
            for name in &names {
                if !is_known_extension(name) {
                    return Err(Error::UndefinedExtension(name.clone()));
                }
            }
        }
        self.parse_drop_behavior()?;
        Ok(Plan::Noop {
            tag: NoopCommand::DropExtension,
        })
    }

    /// Parse `CREATE TYPE name AS ENUM ('a', 'b', …)` into a [`Plan::CreateType`].
    /// (`ENUM` is a contextual identifier, not a reserved keyword.)
    fn parse_create_type(&mut self) -> Result<Plan> {
        self.expect_ident("type")?;
        let name = self.parse_object_name()?;
        self.ts.expect_kw(Keyword::As)?;
        if !self.at_ident("enum") {
            return Err(Error::NotImplemented(
                "CREATE TYPE ... AS (composite) not supported; only AS ENUM".into(),
            ));
        }
        self.expect_ident("enum")?;
        self.ts.expect(&TokenKind::LParen)?;
        let members = self.comma_list_until(&TokenKind::RParen, |p| {
            p.expect_string_literal("an enum label")
        })?;
        self.ts.expect(&TokenKind::RParen)?;
        Ok(Plan::CreateType { name, members })
    }

    /// Parse `CREATE TABLE [IF NOT EXISTS] name ( column_def | table_constraint , … )`
    /// into a [`Plan::CreateTable`].
    ///
    /// The elements are kept in **one written-order list** ([`TableElement`]) and
    /// folded in a single pass, so a table-level constraint written before a column's
    /// inline one lands ahead of it in `unique_constraints`/`checks`/`foreign_keys` —
    /// Postgres's `transformCreateStmt` order, and the order the executor mints names
    /// in. Two sinks concatenated after the fact would silently pair an auto-name with
    /// the wrong specification for the mixed arrangement.
    ///
    /// A table-level `PRIMARY KEY` also forces its key columns NOT NULL, which cannot
    /// ride the same pass: the constraint may name a column declared *later*
    /// (`CREATE TABLE t (PRIMARY KEY (a), a INT)`). It is a per-column flag, not an
    /// ordering, so it is applied afterwards over the collected names.
    fn parse_create_table(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Table)?;
        let if_not_exists = self.parse_if_not_exists()?;
        let name = self.parse_relation_name()?;
        self.ts.expect(&TokenKind::LParen)?;

        let mut elements: Vec<TableElement> = Vec::new();
        self.comma_list(|p| {
            if p.at_table_constraint_start() {
                elements.push(TableElement::Constraint(p.parse_table_constraint()?));
            } else {
                elements.push(TableElement::Column(Box::new(p.parse_column(&name)?)));
            }
            Ok(())
        })?;
        self.ts.expect(&TokenKind::RParen)?;

        let mut columns: Vec<ColumnDef> = Vec::new();
        let mut checks: Vec<CheckConstraintDef> = Vec::new();
        let mut unique_constraints: Vec<UniqueConstraint> = Vec::new();
        let mut foreign_keys: Vec<ForeignKeyDef> = Vec::new();
        // Key columns of every table-level `PRIMARY KEY`, forced NOT NULL below.
        let mut primary_key_columns: Vec<String> = Vec::new();

        for element in elements {
            match element {
                // A `SERIAL`, an identity column, or a column-level `NOT
                // NULL`/`PRIMARY KEY` makes the column non-nullable.
                TableElement::Column(pc) => {
                    let nullable = pc.nullable();
                    columns.push(ColumnDef {
                        name: pc.name,
                        type_name: pc.type_name,
                        nullable,
                        default: pc.default,
                        implicit_sequence: pc.implicit_sequence,
                        identity: pc.identity,
                        identity_start: pc.identity_start,
                        sequence_name: pc.identity_sequence_name,
                    });
                    unique_constraints.extend(pc.column_uniques);
                    checks.extend(pc.checks);
                    foreign_keys.extend(pc.foreign_key);
                }
                TableElement::Constraint(ParsedTableConstraint::PrimaryKey {
                    name,
                    columns: cols,
                }) => {
                    primary_key_columns.extend(cols.iter().cloned());
                    unique_constraints.push(UniqueConstraint {
                        columns: cols,
                        is_primary: true,
                        name,
                    });
                }
                TableElement::Constraint(ParsedTableConstraint::Unique {
                    name,
                    columns: cols,
                }) => {
                    unique_constraints.push(UniqueConstraint {
                        columns: cols,
                        is_primary: false,
                        name,
                    });
                }
                TableElement::Constraint(ParsedTableConstraint::Check(cc)) => checks.push(cc),
                TableElement::Constraint(ParsedTableConstraint::ForeignKey(fk)) => {
                    foreign_keys.push(fk);
                }
            }
        }

        for c in &primary_key_columns {
            if let Some(cd) = columns.iter_mut().find(|cd| &cd.name == c) {
                cd.nullable = false;
            }
        }

        Ok(Plan::CreateTable {
            name,
            columns,
            checks,
            unique_constraints,
            foreign_keys,
            if_not_exists,
        })
    }

    /// Dispatch an `ALTER …` statement: `ALTER SEQUENCE` or `ALTER TABLE`.
    pub(super) fn parse_alter(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Alter)?;
        if self.ts.eat_kw(Keyword::Sequence)? {
            return self.parse_alter_sequence();
        }
        self.ts.expect_kw(Keyword::Table)?;
        self.parse_alter_table_body()
    }

    /// Parse an `ALTER TABLE [IF EXISTS] [ONLY] name <operation>` body into the matching
    /// `Plan::AlterTable*`, or a [`Plan::AlterTableUnperformed`] for an operation pglet
    /// tolerates as a no-op (`OWNER TO`, …) or refuses (`RENAME`, `DROP CONSTRAINT`, …).
    /// The `ONLY` modifier (no table inheritance in pglet) is accepted and ignored.
    ///
    /// **`IF EXISTS` is carried to the executor, not decided against the catalog here.**
    /// 18.6 skips the statement when no relation holds the name, with a `NOTICE`, ahead
    /// of any analysis of the action (`ProcessUtilitySlow`), and performs it otherwise —
    /// so every plan this returns carries the flag and the executor tests the name where
    /// the action would first read it (`Plan::alter_table_target`). Deciding it against
    /// the schema this parser holds would be wrong inside a `DO` body, whose statements
    /// are planned before any of them runs: `CREATE TABLE t …; ALTER TABLE IF EXISTS t ADD
    /// COLUMN …` would plan the skip. The one skip decided here reads no catalog: a
    /// qualifier naming a schema no relation pglet models can be in
    /// ([`qualifier_holds_no_relation`]), which no statement before this one can change.
    ///
    /// `IF` is an unreserved keyword, so it is also a table name: only an `IF` followed by
    /// `EXISTS` is the clause, which is how `ALTER TABLE if DROP COLUMN b` still names a
    /// table `if` (on 18.6 too — no action begins with `EXISTS`).
    fn parse_alter_table_body(&mut self) -> Result<Plan> {
        let if_exists = if self.ts.at_kw(Keyword::If)
            && matches!(self.ts.peek2_kind(), TokenKind::Keyword(Keyword::Exists))
        {
            self.ts.bump()?;
            self.ts.bump()?;
            true
        } else {
            false
        };
        self.eat_ident("only")?; // `ALTER TABLE ONLY …` — ignored (no inheritance)
        let parts = self.parse_dotted_name()?;
        if if_exists && qualifier_holds_no_relation(&parts) {
            // The operation is still read — a grammar error in it is raised, as 18.6's
            // raw parse raises it — and its plan then discarded for the skip.
            let table = parts[1].clone();
            self.parse_alter_table_operation(table.clone(), if_exists)?;
            return Ok(Plan::AlterTableUnperformed {
                table,
                table_if_exists: true,
                answer: UnperformedAlter::Skipped,
            });
        }
        let table = self.reduce_grammar_name(&parts)?;
        self.parse_alter_table_operation(table, if_exists)
    }

    /// The operation of an `ALTER TABLE` over `table`, the name already read.
    fn parse_alter_table_operation(&mut self, table: String, if_exists: bool) -> Result<Plan> {
        if self.ts.eat_kw(Keyword::Add)? {
            self.parse_alter_table_add(table, if_exists)
        } else if self.ts.eat_kw(Keyword::Drop)? {
            if self.ts.eat_kw(Keyword::Constraint)? {
                // `DROP CONSTRAINT [IF EXISTS] name [CASCADE | RESTRICT]` — the grammar is
                // read, and the drop refused where it would run. The spelling matters for
                // what it skips: `pg_dump --clean --if-exists` emits `ALTER TABLE IF EXISTS
                // ONLY t DROP CONSTRAINT IF EXISTS c` for every constraint, so over a
                // table that is not there it is 18.6's skip, and the refusal is reached
                // only over one that is.
                return self.alter_table_unperformed(
                    table,
                    if_exists,
                    Some("ALTER TABLE … DROP CONSTRAINT is not supported"),
                );
            }
            self.ts.eat_kw(Keyword::Column)?;
            let column_if_exists = self.parse_if_exists()?;
            let column = self.expect_name("a column name")?;
            if self.ts.at(&TokenKind::Comma) {
                return self.alter_table_unperformed(
                    table,
                    if_exists,
                    Some("DROP multiple columns not supported"),
                );
            }
            // `CASCADE` / `RESTRICT` tail. `RESTRICT` is the default, so it folds into
            // `cascade = false`; `CASCADE` reaches the DROP-time dependency resolver.
            let cascade = self.parse_drop_behavior()?;
            Ok(Plan::AlterTableDropColumn {
                table,
                column,
                if_exists: column_if_exists,
                cascade,
                table_if_exists: if_exists,
            })
        } else if self.ts.eat_kw(Keyword::Alter)? {
            self.parse_alter_table_alter_column(table, if_exists)
        } else if self.at_ident("rename") {
            // `ALTER TABLE … RENAME [TO … | [COLUMN] … TO … | CONSTRAINT … TO …]`
            // renames an object — a semantic change pglet doesn't implement. Refused
            // loud (`0A000`) over a relation that exists, instead of the silent no-op the
            // tolerated tail below would make of it (which would report success while a
            // later reference to the new name fails "does not exist").
            self.alter_table_unperformed(
                table,
                if_exists,
                Some("ALTER TABLE … RENAME is not supported"),
            )
        } else {
            // `OWNER TO role` (pglet has no roles) and the other ALTER TABLE operations
            // are tolerated as no-ops — the tail is consumed so a pg_dump clause doesn't
            // trip the "expected ';'" check.
            self.alter_table_unperformed(table, if_exists, None)
        }
    }

    /// Consume the rest of an `ALTER TABLE` statement and return the
    /// [`Plan::AlterTableUnperformed`] for its operation: a tolerated no-op when
    /// `refusal` is `None`, a `0A000` carrying `refusal` otherwise — each reached only
    /// once the executor has found the relation, so an `IF EXISTS` over a name no
    /// relation holds skips as 18.6 skips, whatever the operation.
    ///
    /// The tail is consumed unread, as the tolerated no-op always consumed it, so a
    /// grammar error inside it is not raised (`EDGES.md` row 15).
    fn alter_table_unperformed(
        &mut self,
        table: String,
        if_exists: bool,
        refusal: Option<&'static str>,
    ) -> Result<Plan> {
        while !self.ts.at_eof() && !self.ts.at(&TokenKind::Semicolon) {
            self.ts.bump()?;
        }
        Ok(Plan::AlterTableUnperformed {
            table,
            table_if_exists: if_exists,
            answer: refusal.map_or(UnperformedAlter::Tolerated, UnperformedAlter::Refused),
        })
    }

    /// Parse the tail of `ALTER TABLE … ADD …`: a table constraint (`PRIMARY KEY` /
    /// `UNIQUE` / `FOREIGN KEY` / `CHECK`) or `ADD [COLUMN] column_def`.
    fn parse_alter_table_add(&mut self, table: String, table_if_exists: bool) -> Result<Plan> {
        if self.at_table_constraint_start() {
            // Reuse the CREATE TABLE constraint parser, then lower to the
            // ADD-CONSTRAINT plan (PK/UNIQUE/FK build real enforcement; CHECK keeps
            // its own arm).
            let parsed = self.parse_table_constraint()?;
            return Ok(lower_alter_add_constraint(table, table_if_exists, parsed));
        }
        // `ADD [COLUMN] [IF NOT EXISTS] column_def`. The added column's inline
        // `REFERENCES` FK, `UNIQUE`/`PRIMARY KEY`, and `CHECK` are all carried through
        // to the executor, which builds the same enforcement a `CREATE TABLE` inline
        // constraint would (after backfilling any DEFAULT).
        self.ts.eat_kw(Keyword::Column)?;
        let if_not_exists = self.parse_if_not_exists()?;
        let pc = self.parse_column(&table)?;
        let nullable = pc.nullable();
        Ok(Plan::AlterTableAddColumn {
            table,
            column: ColumnDef {
                name: pc.name,
                type_name: pc.type_name,
                nullable,
                default: pc.default,
                implicit_sequence: pc.implicit_sequence,
                identity: pc.identity,
                identity_start: pc.identity_start,
                sequence_name: pc.identity_sequence_name,
            },
            foreign_key: pc.foreign_key,
            column_uniques: pc.column_uniques,
            checks: pc.checks,
            if_not_exists,
            table_if_exists,
        })
    }

    /// Parse the tail of `ALTER TABLE … ALTER [COLUMN] col …`. `ADD GENERATED …
    /// AS IDENTITY [(…)]` makes the column an identity column (the pg_dump form);
    /// `SET DEFAULT expr` / `DROP DEFAULT` rewrite the column's catalog default (the
    /// pg_dump-restore path re-attaches a SERIAL column's `nextval` default after the
    /// data load); `SET NOT NULL` is **enforced** (the executor validates existing
    /// rows + flips the flag). `DROP NOT NULL` and `… TYPE …` are semantically
    /// meaningful but unimplemented, so they **fail loud** rather than silently no-op;
    /// the cosmetic `SET STATISTICS` / `SET (storage …)` hints stay tolerated no-ops.
    fn parse_alter_table_alter_column(
        &mut self,
        table: String,
        table_if_exists: bool,
    ) -> Result<Plan> {
        self.ts.eat_kw(Keyword::Column)?; // optional COLUMN noise word
        let column = self.expect_name("a column name")?;
        // `ADD GENERATED { ALWAYS | BY DEFAULT } AS IDENTITY [( opts )]` — the form
        // pg_dump emits for an identity column (not the inline `CREATE TABLE` form).
        if self.ts.eat_kw(Keyword::Add)? {
            self.expect_ident("generated")?;
            let (generation, options) = self.parse_generated_identity()?;
            return Ok(Plan::AlterTableAddIdentity {
                table,
                column,
                generation,
                start: options.start.unwrap_or(1),
                sequence_name: options.sequence_name,
                table_if_exists,
            });
        }
        if self.ts.eat_kw(Keyword::Set)? {
            if self.ts.eat_kw(Keyword::Default)? {
                // `alter_column_default: SET DEFAULT a_expr` — the full expression
                // grammar, unlike a column constraint's `DEFAULT b_expr`, so the
                // `b_expr` mode stays off here.
                let default = self.parse_column_default(false)?;
                return Ok(Plan::AlterTableSetDefault {
                    table,
                    column,
                    default,
                    table_if_exists,
                });
            }
            if self.ts.eat_kw(Keyword::Not)? {
                self.ts.expect_kw(Keyword::Null)?;
                // Enforced (not a no-op): the executor validates existing rows are
                // non-NULL (else 23502) and flips the column's NOT NULL flag.
                return Ok(Plan::AlterTableSetNotNull {
                    table,
                    column,
                    table_if_exists,
                });
            }
            // `SET DATA TYPE …` is a type change — fail loud, not a silent no-op.
            if self.eat_ident("data")? {
                return self.alter_table_unperformed(
                    table,
                    table_if_exists,
                    Some("ALTER COLUMN … SET DATA TYPE not supported"),
                );
            }
            // `SET STATISTICS …` / `SET (storage = …)` / `SET COMPRESSION …` /
            // `SET STORAGE …` — planner/storage hints with no data effect; no-op.
            return self.alter_table_unperformed(table, table_if_exists, None);
        }
        if self.ts.eat_kw(Keyword::Drop)? {
            if self.ts.eat_kw(Keyword::Default)? {
                return Ok(Plan::AlterTableSetDefault {
                    table,
                    column,
                    default: None,
                    table_if_exists,
                });
            }
            // `DROP NOT NULL` / `DROP IDENTITY` / `DROP EXPRESSION` change column
            // semantics and are unimplemented — fail loud, not a silent no-op.
            return self.alter_table_unperformed(
                table,
                table_if_exists,
                Some("ALTER COLUMN … DROP NOT NULL/IDENTITY/EXPRESSION not supported"),
            );
        }
        // `ALTER COLUMN … TYPE …` (and any other column action) is a type/semantic
        // change pglet doesn't implement — fail loud, not a silent no-op.
        self.alter_table_unperformed(
            table,
            table_if_exists,
            Some("ALTER COLUMN … TYPE not supported"),
        )
    }

    /// Consume the rest of the current statement (up to a top-level `;` or EOF) and
    /// return a [`Plan::Noop`] carrying `tag`. The lexer treats a `;` inside a
    /// string / dollar-quote as part of that token, so this stops only at a real
    /// statement boundary. Used for the pg_dump-preamble tolerance statements
    /// (`GRANT`/`REVOKE`/`COMMENT`/…); `ALTER TABLE`'s tolerated clauses take
    /// [`Self::alter_table_unperformed`] instead, which keeps the relation's name.
    pub(super) fn parse_noop(&mut self, tag: NoopCommand) -> Result<Plan> {
        while !self.ts.at_eof() && !self.ts.at(&TokenKind::Semicolon) {
            self.ts.bump()?;
        }
        Ok(Plan::Noop { tag })
    }

    /// Parse an optionally-signed integer literal (`5`, `-9223372036854775808`) for a
    /// sequence option value.
    fn parse_signed_int_literal(&mut self) -> Result<i64> {
        let negative = self.ts.eat(&TokenKind::Minus)?;
        let tok = self.ts.advance()?;
        if !matches!(tok.kind, TokenKind::Number) {
            return Err(self.ts.expected("an integer"));
        }
        let raw = self.ts.text(&tok);
        let magnitude = raw
            .parse::<i64>()
            .map_err(|_| self.ts.error(&format!("invalid integer: {raw}")))?;
        Ok(if negative { -magnitude } else { magnitude })
    }

    /// Parse a `CREATE`/`ALTER SEQUENCE … OWNED BY` target: a dotted
    /// `[schema.]table.column` or the literal `NONE`.
    ///
    /// The last two components are the table and column; a leading schema qualifier
    /// goes through [`super::reduce_qualified_name`], which accepts `public` and
    /// rejects any other schema — the same rule every other qualified name follows.
    /// A bare one-part name is a syntax error (Postgres requires `table.column`).
    fn parse_owned_by(&mut self) -> Result<SequenceOwnerSpec> {
        let mut parts = vec![self.expect_name("a sequence owner column or NONE")?];
        while self.ts.eat(&TokenKind::Dot)? {
            parts.push(self.expect_name("a name after '.'")?);
        }
        if parts.len() == 1 {
            // `NONE` is a grammar keyword in `AlterSeqStmt`, not a name, so a quoted
            // `"NONE"` is an ordinary one-part name — `42601 invalid OWNED BY option`
            // on 18.6, and the same `42601` here. `expect_name` downcased the
            // unquoted spelling, so this compares exactly.
            if parts[0] == "none" {
                return Ok(SequenceOwnerSpec::None);
            }
            return Err(self.ts.expected("OWNED BY <table>.<column> or NONE"));
        }
        let Some(column) = parts.pop() else {
            return Err(self.ts.expected("OWNED BY <table>.<column> or NONE"));
        };
        let table = super::reduce_qualified_name(&parts)?;
        Ok(SequenceOwnerSpec::Column { table, column })
    }

    /// Whether the current token begins a table-level constraint (`CONSTRAINT`,
    /// `PRIMARY`, `UNIQUE`, `CHECK`, `FOREIGN`) rather than a column definition.
    fn at_table_constraint_start(&self) -> bool {
        self.ts.at_kw(Keyword::Constraint)
            || self.ts.at_kw(Keyword::Primary)
            || self.ts.at_kw(Keyword::Unique)
            || self.ts.at_kw(Keyword::Check)
            || self.ts.at_kw(Keyword::Foreign)
    }

    /// Parse one column definition (`name type [column_constraint …]`).
    fn parse_column(&mut self, table: &str) -> Result<ParsedColumn> {
        let name = self.expect_name("a column name")?;
        let type_name = self.parse_type_name()?;
        let is_serial = is_serial_type(&type_name);

        let mut not_null = false;
        let mut has_primary = false;
        let mut column_uniques = Vec::new();
        let mut default = None;
        let mut checks = Vec::new();
        let mut foreign_key = None;
        let mut identity = None;
        let mut identity_options = IdentityOptions::default();
        // `transformColumnDefinition`'s `saw_default`. A `SERIAL` type name seeds it
        // because that spelling *is* a default clause there (it synthesizes the
        // `nextval`), which is why the flag is not simply `default.is_some()`.
        let mut saw_default = is_serial;

        loop {
            // A column-level `CONSTRAINT name` prefix. The name is kept for the four
            // constraint kinds pglet stores by name — `PRIMARY KEY` / `UNIQUE` (where
            // it becomes the constraint's, and so its index's, name), `CHECK`, and
            // `REFERENCES` — and dropped only for `NOT NULL`, which PG 15+ does name. A
            // dropped name is not inert: it is invisible in `information_schema` *and*
            // collides with nothing, so the same name could be reused on the same table.
            let declared_name = if self.ts.eat_kw(Keyword::Constraint)? {
                Some(self.expect_name("a constraint name")?)
            } else {
                None
            };
            let named = declared_name.is_some();

            if self.ts.eat_kw(Keyword::Not)? {
                self.ts.expect_kw(Keyword::Null)?;
                not_null = true;
            } else if self.ts.eat_kw(Keyword::Null)? {
                // explicit `NULL` — a no-op (the column is nullable by default).
            } else if self.ts.eat_kw(Keyword::Primary)? {
                self.ts.expect_kw(Keyword::Key)?;
                has_primary = true;
                column_uniques.push(UniqueConstraint {
                    columns: vec![name.clone()],
                    is_primary: true,
                    name: declared_name,
                });
            } else if self.ts.eat_kw(Keyword::Unique)? {
                column_uniques.push(UniqueConstraint {
                    columns: vec![name.clone()],
                    is_primary: false,
                    name: declared_name,
                });
            } else if self.ts.eat_kw(Keyword::Default)? {
                // A column carries **one** default, and a `SERIAL` type name is one
                // of the two ways to spell it — `transformColumnDefinition` sets
                // `saw_default` for the implicit `nextval` it synthesizes, so
                // `a SERIAL DEFAULT 5` is the same `42601` as `a INT DEFAULT 5
                // DEFAULT 7` (both measured on 18.6, message included). A
                // last-wins assignment silently kept the second and dropped the
                // first, which for the `SERIAL` spelling also dropped the implicit
                // sequence the type name asked for.
                // Both refusals are `transformColumnDefinition`'s, which 18.6 runs
                // over every column before it transforms any expression in the
                // statement — so each outranks a too-deep chain before it.
                if saw_default {
                    return Err(self.before_statement_analysis(Error::syntax(format!(
                        "multiple default values specified for column \"{name}\" of table \"{table}\""
                    ))));
                }
                // `GENERATED … AS IDENTITY` is not a default and takes its own
                // sentence, in either written order (measured on 18.6: the identity
                // clause tests `saw_default` and this clause tests `saw_identity`).
                if identity.is_some() {
                    return Err(
                        self.before_statement_analysis(default_and_identity_conflict(&name, table))
                    );
                }
                saw_default = true;
                // `ColConstraintElem: DEFAULT b_expr` — reached by `CREATE TABLE` and
                // `ALTER TABLE … ADD COLUMN` alike, and the one route that takes the
                // restricted grammar (see the parser's `b_expr` field).
                default = self.parse_column_default(true)?;
            } else if self.ts.eat_kw(Keyword::Check)? {
                self.ts.expect(&TokenKind::LParen)?;
                let expr = self.parse_expr_bp(0)?;
                self.ts.expect(&TokenKind::RParen)?;
                reject_persisted_expr_constructs(&expr, PersistedExprContext::Check)?;
                checks.push(CheckConstraintDef {
                    name: declared_name,
                    expr,
                });
            } else if self.ts.eat_kw(Keyword::References)? {
                foreign_key = self.parse_inline_references(&name, declared_name)?;
            } else if self.eat_ident("generated")? {
                let (generation, options) = self.parse_generated_identity()?;
                // The other half of the identity/default exclusion — the written
                // order decides which clause raises, not which sentence is used.
                if saw_default {
                    return Err(
                        self.before_statement_analysis(default_and_identity_conflict(&name, table))
                    );
                }
                identity = Some(generation);
                identity_options = options;
            } else if named {
                return Err(self
                    .ts
                    .expected("a column constraint after CONSTRAINT name"));
            } else {
                break;
            }
        }

        // `SERIAL` and `GENERATED … AS IDENTITY` both supply an implicit sequence
        // default (an identity column *is* a SERIAL plus the `428C9` rule), overriding
        // any DEFAULT clause (serial-first order).
        //
        // The *name* of that sequence is deliberately **not** decided here. It is the
        // engine's name, so it has to be disambiguated against the live relation
        // namespace rather than refused (`t_a_seq`, `t_a_seq1`, …), which a parser
        // holding no catalog cannot do — and a `String` minted here cannot even say
        // "the engine chose this", the distinction the whole naming rule turns on. So
        // the parse side says *implicit-sequence* and the executor mints
        // (`mint_sequence_name`); a user's own `DEFAULT nextval('x')` keeps its name
        // in `default` and is never rewritten.
        let implicit_sequence = is_serial || identity.is_some();
        if implicit_sequence {
            default = None;
        }

        Ok(ParsedColumn {
            name,
            type_name,
            not_null,
            has_primary,
            column_uniques,
            default,
            implicit_sequence,
            checks,
            foreign_key,
            identity,
            identity_start: identity_options.start,
            identity_sequence_name: identity_options.sequence_name,
        })
    }

    /// Parse the tail of a `GENERATED …` column constraint (the leading `generated`
    /// ident is already consumed): `{ ALWAYS | BY DEFAULT } AS { IDENTITY [( seq
    /// options )] | ( <expr> ) STORED }`.
    ///
    /// Returns the identity generation and its [`IdentityOptions`]. A `GENERATED …
    /// AS ( <expr> ) STORED` **computed column** is recognized and rejected cleanly
    /// with `0A000` (not modeled) rather than falling through to a raw parse error.
    fn parse_generated_identity(&mut self) -> Result<(IdentityGeneration, IdentityOptions)> {
        // `ALWAYS` (contextual ident) | `BY DEFAULT` (keywords).
        let generation = if self.eat_ident("always")? {
            IdentityGeneration::Always
        } else if self.ts.eat_kw(Keyword::By)? {
            self.ts.expect_kw(Keyword::Default)?;
            IdentityGeneration::ByDefault
        } else {
            return Err(self.ts.expected("ALWAYS or BY DEFAULT"));
        };
        self.ts.expect_kw(Keyword::As)?;
        if self.eat_ident("identity")? {
            // Optional parenthesized sequence-option list (`START WITH n …`).
            let options = if self.ts.at(&TokenKind::LParen) {
                self.parse_identity_options()?
            } else {
                IdentityOptions::default()
            };
            Ok((generation, options))
        } else if self.ts.at(&TokenKind::LParen) {
            // `GENERATED … AS ( <expr> ) STORED` — a computed/stored column. Consume
            // the expression + optional `STORED` so the rejection is clean, then fail
            // loud (pglet models no stored generated columns).
            self.ts.expect(&TokenKind::LParen)?;
            self.parse_expr_bp(0)?;
            self.ts.expect(&TokenKind::RParen)?;
            self.eat_ident("stored")?;
            Err(Error::NotImplemented(
                "GENERATED … STORED columns are not supported".into(),
            ))
        } else {
            Err(self.ts.expected("IDENTITY or ( expression ) STORED"))
        }
    }

    /// Parse an identity column's parenthesized sequence-option list.
    ///
    /// pglet models a sequence as a counter plus its start and owner, so `START
    /// [WITH] n` and `SEQUENCE NAME <n>` are honored; every other option (`INCREMENT
    /// [BY]`, `[NO] MINVALUE/MAXVALUE`, `CACHE`, `[NO] CYCLE`, `AS <type>`, `OWNED BY
    /// …`) is parsed and its value discarded. Options are space-separated (no commas),
    /// mirroring `CREATE SEQUENCE`; an unrecognized option or a missing `)` fails loud.
    ///
    /// `AS <type>` still reads a real type name, so a refusal the *grammar* raises
    /// escapes the discard — a `FLOAT(n)` outside 1–53 is `22023` here as it is on
    /// 18.6, where `opt_float` decides. A refusal that needs the *resolved* type does
    /// not escape: the in-range spellings stay in the dropped-clause divergence
    /// (`EDGES.md` row 116; `executor::resolve_column_type`'s doc measures the set).
    fn parse_identity_options(&mut self) -> Result<IdentityOptions> {
        self.ts.expect(&TokenKind::LParen)?;
        let mut options = IdentityOptions::default();
        loop {
            if self.ts.at(&TokenKind::RParen) {
                break;
            }
            if self.ts.eat_kw(Keyword::Start)? {
                self.ts.eat_kw(Keyword::With)?; // optional WITH
                options.start = Some(self.parse_signed_int_literal()?);
            } else if self.ts.eat_kw(Keyword::As)? {
                // AS <data type> — the name is read (so `opt_float`'s `22023` escapes)
                // and the resolved type discarded
                self.parse_type_name()?;
            } else if self.eat_ident("increment")? {
                self.ts.eat_kw(Keyword::By)?; // optional BY
                self.parse_signed_int_literal()?; // ignored
            } else if self.ts.eat_kw(Keyword::No)? {
                // NO MINVALUE / NO MAXVALUE / NO CYCLE.
                if !(self.eat_ident("minvalue")?
                    || self.eat_ident("maxvalue")?
                    || self.eat_ident("cycle")?)
                {
                    return Err(self.ts.expected("MINVALUE, MAXVALUE, or CYCLE after NO"));
                }
            } else if self.eat_ident("minvalue")?
                || self.eat_ident("maxvalue")?
                || self.eat_ident("cache")?
            {
                self.parse_signed_int_literal()?; // ignored
            } else if self.eat_ident("cycle")? {
                // CYCLE — ignored.
            } else if self.ts.eat_kw(Keyword::Sequence)? {
                // `SEQUENCE NAME <name>` — the user's *declared* name for the implicit
                // sequence, and the form pg_dump emits. Carried through verbatim: the
                // engine's own name is disambiguated, a declared one is taken as
                // written (and refused, `42P07`, if the relation namespace holds it).
                // Discarding it silently re-minted `<t>_<c>_seq`, which coincides with
                // pg_dump's usual choice and so looked harmless until the two differ —
                // at which point the restore's `setval` names a sequence the catalog
                // does not have and the identity counter silently restarts at 1.
                self.expect_ident("name")?;
                // `SeqOptElem: SEQUENCE NAME_P any_name` — any number of parts in the
                // grammar; `transformColumnDefinition` refuses a four-part one
                // (`makeRangeVarFromNameList`), ahead of any expression of the
                // statement, and nothing earlier.
                let parts = self.parse_dotted_name()?;
                let name = super::reduce_qualified_name(&parts).map_err(|e| {
                    if parts.len() >= 4 {
                        self.before_statement_analysis(e)
                    } else {
                        e
                    }
                })?;
                options.sequence_name = Some(name);
            } else if self.eat_ident("owned")? {
                // `OWNED BY` inside an identity column's option list: parsed and
                // discarded, because an identity sequence is owned by its own column
                // by construction — the executor records that owner regardless.
                self.ts.expect_kw(Keyword::By)?;
                self.parse_owned_by()?;
            } else {
                return Err(self.ts.expected("a sequence option or ')'"));
            }
        }
        self.ts.expect(&TokenKind::RParen)?;
        Ok(options)
    }

    /// Parse an inline column-level `REFERENCES parent [( col )] [MATCH …] [ON DELETE
    /// …] [ON UPDATE …]`, carrying `declared_name` (the `CONSTRAINT <n>` prefix) through
    /// unchanged — an inline `REFERENCES` is nameable like any other column constraint
    /// (`CREATE TABLE t (a INT CONSTRAINT myfk REFERENCES p(id))` is `myfk` on
    /// Postgres). An unnamed one is left `None`; the executor mints
    /// `<table>_<column>_fkey`, disambiguated, at execute time.
    ///
    /// An omitted parent-column list resolves to the parent's PRIMARY KEY at execute
    /// time (`parent_columns` left empty here). A parent list with more than one
    /// column can't pair with the single inline column, so it fails loud (`42830`).
    fn parse_inline_references(
        &mut self,
        column: &str,
        declared_name: Option<String>,
    ) -> Result<Option<ForeignKeyDef>> {
        let parent_table = self.parse_relation_name()?;
        let parent_cols = self.parse_optional_paren_name_list()?;
        self.parse_fk_match()?;
        let (on_delete, on_update) = self.parse_ref_actions()?;
        // Inline is single-column; a parent list must be empty (use the PK) or one.
        if parent_cols.len() > 1 {
            return Err(Error::ForeignKeyColumnMismatch);
        }
        Ok(Some(ForeignKeyDef {
            name: declared_name,
            columns: vec![column.to_string()],
            parent_table,
            parent_columns: parent_cols,
            on_delete,
            on_update,
        }))
    }

    /// Parse one table-level constraint (an optional `CONSTRAINT name` prefix then
    /// `PRIMARY KEY` / `UNIQUE` / `CHECK` / `FOREIGN KEY`, single- or multi-column).
    fn parse_table_constraint(&mut self) -> Result<ParsedTableConstraint> {
        let name = if self.ts.eat_kw(Keyword::Constraint)? {
            Some(self.expect_name("a constraint name")?)
        } else {
            None
        };

        if self.ts.eat_kw(Keyword::Primary)? {
            self.ts.expect_kw(Keyword::Key)?;
            Ok(ParsedTableConstraint::PrimaryKey {
                name,
                columns: self.parse_paren_name_list()?,
            })
        } else if self.ts.eat_kw(Keyword::Unique)? {
            Ok(ParsedTableConstraint::Unique {
                name,
                columns: self.parse_paren_name_list()?,
            })
        } else if self.ts.eat_kw(Keyword::Check)? {
            self.ts.expect(&TokenKind::LParen)?;
            let expr = self.parse_expr_bp(0)?;
            self.ts.expect(&TokenKind::RParen)?;
            reject_persisted_expr_constructs(&expr, PersistedExprContext::Check)?;
            Ok(ParsedTableConstraint::Check(CheckConstraintDef {
                name,
                expr,
            }))
        } else if self.ts.eat_kw(Keyword::Foreign)? {
            self.ts.expect_kw(Keyword::Key)?;
            let local = self.parse_paren_name_list()?;
            self.ts.expect_kw(Keyword::References)?;
            let parent_table = self.parse_relation_name()?;
            let parent_cols = self.parse_optional_paren_name_list()?;
            self.parse_fk_match()?;
            let (on_delete, on_update) = self.parse_ref_actions()?;
            // An explicit parent list must match the local list's arity (Postgres
            // `42830`); an omitted list resolves to the parent PK at execute time.
            if !parent_cols.is_empty() && parent_cols.len() != local.len() {
                return Err(Error::ForeignKeyColumnMismatch);
            }
            // The name stays as written (`None` when undeclared) — the executor mints
            // `<table>_<col1>_<col2>_…_fkey` and disambiguates it there.
            Ok(ParsedTableConstraint::ForeignKey(ForeignKeyDef {
                name,
                columns: local,
                parent_table,
                parent_columns: parent_cols,
                on_delete,
                on_update,
            }))
        } else {
            Err(self.ts.expected("a table constraint"))
        }
    }

    /// Consume an optional `MATCH { SIMPLE | FULL | PARTIAL }` clause of a foreign
    /// key. `MATCH SIMPLE` (the default) is what pglet implements; `MATCH FULL` /
    /// `MATCH PARTIAL` change the NULL-handling semantics, so pglet **fails loud**
    /// (`0A000`) rather than silently treating them as SIMPLE. `MATCH` is a
    /// contextual identifier (unreserved), recognized only in this FK position.
    fn parse_fk_match(&mut self) -> Result<()> {
        if !self.eat_ident("match")? {
            return Ok(());
        }
        if self.eat_ident("simple")? {
            Ok(())
        } else if self.ts.at_kw(Keyword::Full) || self.at_ident("partial") {
            // `FULL` is a reserved keyword (FULL OUTER JOIN); `PARTIAL` is a
            // contextual identifier — both fail loud here.
            Err(Error::NotImplemented(
                "MATCH FULL / MATCH PARTIAL foreign keys are not supported (only MATCH SIMPLE)"
                    .into(),
            ))
        } else {
            Err(self.ts.expected("SIMPLE, FULL, or PARTIAL after MATCH"))
        }
    }

    /// Parse the trailing `[ON DELETE action] [ON UPDATE action]` of a foreign key
    /// (either order, at most one of each), returning `(on_delete, on_update)` — each
    /// defaulting to `NO ACTION`.
    fn parse_ref_actions(&mut self) -> Result<(ReferentialAction, ReferentialAction)> {
        let mut on_delete = ReferentialAction::NoAction;
        let mut on_update = ReferentialAction::NoAction;
        while self.ts.at_kw(Keyword::On) {
            if matches!(self.ts.peek2_kind(), TokenKind::Keyword(Keyword::Delete)) {
                self.ts.bump()?; // ON
                self.ts.bump()?; // DELETE
                on_delete = self.parse_referential_action()?;
            } else if matches!(self.ts.peek2_kind(), TokenKind::Keyword(Keyword::Update)) {
                self.ts.bump()?; // ON
                self.ts.bump()?; // UPDATE
                on_update = self.parse_referential_action()?;
            } else {
                break;
            }
        }
        Ok((on_delete, on_update))
    }

    /// Parse a referential action (`CASCADE` / `RESTRICT` / `SET NULL` / `SET
    /// DEFAULT` / `NO ACTION`). `SET DEFAULT` **fails loud** (`0A000`): pglet
    /// doesn't reset a child's FK to its column default, and silently substituting
    /// `NO ACTION` (which instead *rejects* the parent change) would be a
    /// different, wrong behavior. Shared by the `ON DELETE` and `ON UPDATE` sides.
    /// Non-spine.
    fn parse_referential_action(&mut self) -> Result<ReferentialAction> {
        if self.ts.eat_kw(Keyword::Cascade)? {
            Ok(ReferentialAction::Cascade)
        } else if self.ts.eat_kw(Keyword::Restrict)? {
            Ok(ReferentialAction::Restrict)
        } else if self.ts.eat_kw(Keyword::Set)? {
            if self.ts.eat_kw(Keyword::Null)? {
                Ok(ReferentialAction::SetNull)
            } else {
                self.ts.expect_kw(Keyword::Default)?;
                Err(Error::NotImplemented(
                    "SET DEFAULT referential action not supported".into(),
                ))
            }
        } else if self.ts.eat_kw(Keyword::No)? {
            self.ts.expect_kw(Keyword::Action)?;
            Ok(ReferentialAction::NoAction)
        } else {
            Err(self.ts.expected("a referential action"))
        }
    }

    /// Parse a column `DEFAULT` into a [`ColumnDefault`] — one of the ten recognized
    /// closed forms, or the [`ColumnDefault::Expr`] catch-all for everything else.
    ///
    /// **Almost no parse-analysis verdict is decided here.** A subquery, an aggregate, a
    /// window, a set-returning function, a column reference, a `$N` and a bare `DEFAULT`
    /// are all the executor's (`executor::ddl`'s `cook_column_default`), because 18.6
    /// raises them behind the statement-level gates a parse-time refusal would jump in
    /// front of. What this function refuses itself is what the grammar cannot read at all
    /// (a malformed expression, and the `nextval` arm's non-literal argument), which is
    /// bison's position on 18.6 too. Otherwise its whole job is to produce a tree and to
    /// normalize it.
    ///
    /// The **set quantifier** is the exception, at both of this site's routes — the
    /// general path's `planner::build_call` and the classifier's
    /// [`Parser::parse_nextval_arg`] — and it is a residual rather than a design: 18.6
    /// raises `42809` for it while *cooking* the default, so `DEFAULT (upper(DISTINCT
    /// 'a'))` and `DEFAULT nextval(DISTINCT 's')` are refused here ahead of the column
    /// type's `42704`, the table's own `42P07` and a later syntax error, each of which
    /// outranks them there. That is the recorded parse-time-precedence class this engine
    /// already carries (`EDGES.md`, rows 47/48/51), not a new
    /// one — and refusing the quantifier at only one of the two routes was worse, because
    /// then the two spellings of one default disagreed with each other.
    /// **The quantifier brings 18.6's argument transform with it**, which is one exception
    /// and not two: a quantified call's constant cast is coerced here as well
    /// (`planner::transform_call_args`, gated on a written quantifier), so
    /// `DEFAULT upper(DISTINCT 'a'::int)` is `22P02` here ahead of those same gates — as
    /// it is on 18.6, ahead of the quantifier. Its **unquantified** twin is decided
    /// nowhere near here: measured, `CREATE TABLE IF NOT EXISTS <existing> (b TEXT DEFAULT
    /// upper('a'::int))` is a skip on both engines, and running that transform for an
    /// unquantified call makes it `22P02`, along with four more such skips
    /// and twelve unquantified DDL statements whose own error codes it jumped.
    ///
    /// **Two phases, and the split is what keeps the recognized forms
    /// byte-identical.**
    ///
    /// 1. [`Self::try_recognized_default`] classifies the leading tokens, then the
    ///    operator loop is *continued* from the equivalent `Expr`
    ///    ([`column_default_to_expr`]). Consuming nothing means the default was the
    ///    whole recognized form and it is returned unchanged. The phase now exists
    ///    for exactly **one** spelling: `nextval('seq'::regclass)`, pg_dump's form
    ///    and pglet's own dump's, which the general grammar refuses: `parse_cast_target`
    ///    lowers the `regclass` cast to a `StringFn` that
    ///    `planner::sequence_name_from_arg`'s look-through does not match, so it answers
    ///    `0A000 not implemented: nextval() requires a string-literal sequence name`.
    ///    Its *parenthesized* spelling `DEFAULT (nextval('s'::regclass))` leaves the
    ///    classifier and is still that `0A000`, where 18.6 treats the two as identical
    ///    (measured: `1` and an accepted `CREATE TABLE` there) — a loud over-rejection
    ///    recorded in `EDGES.md`, reached by no dump (both
    ///    emitters write the bare form).
    /// 2. Otherwise the expression is parsed in full (or continued from phase 1's
    ///    seed) and [`column_default_from_expr`] normalizes an exactly-recognized
    ///    tree back onto its closed arm — which is what makes `DEFAULT (nextval('s'))`
    ///    and `DEFAULT nextval('s')` the same stored fact, as they are on 18.6.
    ///
    /// Both phases end at [`without_null_default`], because a `DEFAULT` whose cooked
    /// expression is a NULL constant **is** no default — 18.6 stores no `pg_attrdef`
    /// row for it, and `SET DEFAULT NULL` removes one.
    ///
    /// `b_expr` selects the grammar: `true` for a column constraint's `DEFAULT`,
    /// `false` (full `a_expr`) for `ALTER COLUMN … SET DEFAULT`. The two callers
    /// genuinely differ in `gram.y`; see the parser's `b_expr` field. It is
    /// save/restored around the parse, error path included, because a `DEFAULT` sits
    /// inside a column list the caller keeps reading.
    ///
    /// A **negative** numeric literal is folded into the constant by
    /// `parse_prefix`'s `negate` and a **positive** one keeps its
    /// [`Expr::UnaryPlus`] wrapper, matching `gram.y`'s `doNegate` asymmetry — but
    /// [`column_default_from_expr`] then folds the `+` away too, so both signs store
    /// a bare `Value`. That is observable through
    /// `information_schema.columns.column_default`, where 18.6 renders `DEFAULT +7`
    /// as `(+ 7)` and `DEFAULT -7` as `'-7'::integer` against pglet's `7` and `-7` —
    /// a recorded rendering divergence, and *not* the unobservable one an earlier
    /// note here claimed (it argued the only observable was the SQL92 ordinal rule in
    /// `GROUP BY`/`ORDER BY`, which a column `DEFAULT` indeed cannot reach; that is
    /// simply not the only one).
    fn parse_column_default(&mut self, b_expr: bool) -> Result<Option<ColumnDefault>> {
        let outer_b_expr = std::mem::replace(&mut self.b_expr, b_expr);
        let parsed = self.parse_column_default_expr();
        self.b_expr = outer_b_expr;
        parsed
    }

    /// [`Self::parse_column_default`]'s body, run with the `b_expr` mode already
    /// set — split out so the mode is restored on the error path too.
    fn parse_column_default_expr(&mut self) -> Result<Option<ColumnDefault>> {
        if let Some(recognized) = self.try_recognized_default()? {
            let seed = column_default_to_expr(&recognized);
            let (expr, extended) = self.continue_expr_bp(seed, 0)?;
            if !extended {
                return Ok(without_null_default(recognized));
            }
            return Ok(without_null_default(default_from_expr(&expr)));
        }
        let expr = self.parse_expr_bp(0)?;
        Ok(without_null_default(default_from_expr(&expr)))
    }

    /// Classify the leading tokens of a `DEFAULT` as one of the closed
    /// [`ColumnDefault`] forms, consuming them; `None` (consuming nothing beyond a
    /// numeric sign) when they are not one.
    ///
    /// Token-level, not a parse: this is deliberately the *same* classifier the
    /// engine has always run, so every form it accepts keeps its exact stored
    /// [`Value`]. Its caller decides what a trailing continuation means.
    fn try_recognized_default(&mut self) -> Result<Option<ColumnDefault>> {
        if matches!(self.ts.peek_kind(), TokenKind::Identifier)
            && let Some(d) = self.try_function_default()?
        {
            return Ok(Some(d));
        }
        // Not a recognized form, and nothing has been consumed — the caller parses
        // the whole expression from here.
        Ok(None)
    }

    /// Recognize a function-call `DEFAULT` (`now`/`current_timestamp` family,
    /// `gen_random_uuid`/`uuid_generate_v4`, `nextval`). Returns `None` (without
    /// consuming) if the current identifier isn't one of these.
    ///
    /// **An ordinary function's parens are required**, so the arms for the six real
    /// `pg_proc` entries are guarded on the *next-but-one* token being a `(` rather
    /// than consuming the bare identifier. A bare `now` / `clock_timestamp` /
    /// `statement_timestamp` / `transaction_timestamp` / `gen_random_uuid` /
    /// `uuid_generate_v4` is a **column reference** on 18.6 — `0A000 cannot use
    /// column reference in DEFAULT expression`, measured — and consuming it here
    /// silently accepted it as the function's value instead, backfilling a timestamp
    /// or a UUID into a column 18.6 refuses to define. Declining lets it fall through
    /// to the general expression path, where the same column-reference verdict lands
    /// (`executor::ddl`'s `check_default_expr_nodes`); it is also what made the two spellings
    /// disagree with each other, since `DEFAULT (now)` already took that path.
    ///
    /// `CURRENT_TIMESTAMP` / `CURRENT_DATE` / `CURRENT_TIME` are **not** guarded:
    /// they are parenless SQL value functions, legal bare on both engines.
    /// `nextval` needs no guard either — its argument is not optional, so
    /// [`Parser::parse_nextval_arg`] demands the `(` itself. It answers `42601` to
    /// the bare spelling where 18.6 answers the same `0A000` column reference as the
    /// six above; loud either way, and a wrong code rather than a wrong write.
    fn try_function_default(&mut self) -> Result<Option<ColumnDefault>> {
        // The caller only reaches here on a `TokenKind::Identifier`, i.e. an unquoted
        // word, so this fold is `Token::name`'s applied in place. A quoted spelling
        // falls through to the caller's fail-loud arm rather than matching.
        let fname = self.ts.peek_text().to_ascii_lowercase();
        match fname.as_str() {
            // The four `now`-family *functions*, each classified under its own spelling
            // — they are not synonyms to record: 18.6 renders each verbatim in
            // `column_default` and in `pg_dump`, and only `clock_timestamp()` is
            // volatile enough to be re-evaluated per row by an `ADD COLUMN` backfill.
            "now" if *self.ts.peek2_kind() == TokenKind::LParen => {
                self.ts.bump()?;
                self.eat_empty_call()?;
                Ok(Some(ColumnDefault::Now))
            }
            "transaction_timestamp" if *self.ts.peek2_kind() == TokenKind::LParen => {
                self.ts.bump()?;
                self.eat_empty_call()?;
                Ok(Some(ColumnDefault::TransactionTimestamp))
            }
            "statement_timestamp" if *self.ts.peek2_kind() == TokenKind::LParen => {
                self.ts.bump()?;
                self.eat_empty_call()?;
                Ok(Some(ColumnDefault::StatementTimestamp))
            }
            "clock_timestamp" if *self.ts.peek2_kind() == TokenKind::LParen => {
                self.ts.bump()?;
                self.eat_empty_call()?;
                Ok(Some(ColumnDefault::ClockTimestamp))
            }
            // A parenless SQL value function, and the paren guard here points the
            // **opposite** way to the six ordinary functions': theirs requires the
            // `(`, this one refuses it. `current_timestamp()` is
            // `42601 syntax error at or near ")"` on 18.6 — a SQL value function
            // takes no argument list — and `eat_empty_call` used to swallow it, so
            // the column was defined and backfilled a real timestamp where 18.6
            // defines no default at all. Declining hands the spelling to the general
            // grammar, which already answers `42601 current_timestamp takes no
            // argument list` (the route the parenthesized
            // `DEFAULT (current_timestamp())` always took, and the reason the two
            // spellings disagreed with each other).
            "current_timestamp" if *self.ts.peek2_kind() != TokenKind::LParen => {
                self.ts.bump()?;
                Ok(Some(ColumnDefault::CurrentTimestamp))
            }
            // `CURRENT_DATE` is a parenless value function (not a call), so no
            // `eat_empty_call` — `current_date()` is not valid Postgres.
            "current_date" => {
                self.ts.bump()?;
                Ok(Some(ColumnDefault::CurrentDate))
            }
            // `CURRENT_TIME` is likewise a parenless value function.
            "current_time" => {
                self.ts.bump()?;
                Ok(Some(ColumnDefault::CurrentTime))
            }
            "gen_random_uuid" | "uuid_generate_v4"
                if *self.ts.peek2_kind() == TokenKind::LParen =>
            {
                self.ts.bump()?;
                self.eat_empty_call()?;
                Ok(Some(ColumnDefault::RandomUuid))
            }
            "nextval" => {
                self.ts.bump()?;
                // A non-string argument leaves the call already consumed, so a `None`
                // here would resume the caller's parse *after* it rather than at it.
                // Report the same `0A000` the general expression path's
                // `sequence_name_from_arg` raises for the identical shape.
                self.parse_nextval_arg()?.map_or_else(
                    || {
                        Err(Error::NotImplemented(
                            "nextval() requires a string-literal sequence name".into(),
                        ))
                    },
                    |seq| Ok(Some(ColumnDefault::NextVal(seq))),
                )
            }
            _ => Ok(None),
        }
    }

    /// Consume an empty `()` argument list. Every caller in
    /// [`Parser::try_function_default`] is an *ordinary function* arm that has
    /// already checked the `(` is there (the parens are required), so the `if` is
    /// defensive rather than load-bearing — it stopped being load-bearing when the
    /// `CURRENT_TIMESTAMP` arm, the one caller that could arrive with no parens,
    /// was changed to decline that spelling instead of swallowing it (18.6:
    /// `42601`). A non-empty list is `42601` from the `expect`, which is what keeps
    /// `DEFAULT now(1)` loud.
    fn eat_empty_call(&mut self) -> Result<()> {
        if self.ts.eat(&TokenKind::LParen)? {
            self.ts.expect(&TokenKind::RParen)?;
        }
        Ok(())
    }

    /// Parse `( [DISTINCT | ALL] 'seq' [::regclass] )`, returning the sequence name, or
    /// `None` if the argument isn't a string literal.
    ///
    /// **The third route that reads a function call's set quantifier**, and the only one
    /// outside `parser::expr`'s `parse_function_call` and `parser::select`'s
    /// `parse_unnest_table`: this classifier parses the argument list itself, so a
    /// quantifier written here reaches neither of those. It is also the only *member* —
    /// `nextval` is the one arm of [`Parser::try_function_default`] whose call takes an
    /// argument, and the niladic ones sit behind [`Parser::eat_empty_call`]'s
    /// `expect(RParen)`, which answers `42601` to `DEFAULT now(ALL)` / `now(DISTINCT)` /
    /// `gen_random_uuid(ALL)` exactly as 18.6's grammar bound does (measured on both).
    /// Without the read, `DEFAULT nextval(ALL 's')` was `42601` here against a plain
    /// accept there, at all three `DEFAULT` positions — and, worse, disagreed with its
    /// own parenthesized spelling, which leaves the classifier for the general path and
    /// accepted it, against [`Parser::parse_column_default`]'s invariant that the two are
    /// one stored fact.
    ///
    /// **The two positions are the general path's, because the two spellings of one
    /// default must answer alike.** The verdict runs *after* the closing paren, so a
    /// syntax error standing where the `)` should be wins — `DEFAULT nextval(DISTINCT 's'
    /// junk)` is `42601` on both engines, where a verdict above the `expect` would answer
    /// `42809` (the same ordering `parse_unnest_table` and `parse_function_call` carry,
    /// and the same trap). And it runs *before* the name-literal reduction, which is
    /// where `planner::sequence_call` puts it: `DEFAULT nextval(DISTINCT 'bad name')` is
    /// `42809` on 18.6 and on the parenthesized route, not the `42602` the reduction
    /// would answer. That is why the literal is captured here and reduced below — and it
    /// closes an unquantified divergence of the same shape on the way, since the
    /// reduction used to beat the paren too: `DEFAULT nextval('bad name' junk)` was
    /// `42602` here against `42601` on 18.6 and on its own parenthesized spelling.
    /// The condition on that move is that the bad token stands **inside** the call, and it
    /// is worth stating as the condition rather than as a list: the `junk`, `'bad name'::`
    /// and `'bad name'::123` spellings all moved, at each of `CREATE TABLE`, `ADD COLUMN`
    /// and `SET DEFAULT`, as did the unterminated `nextval('bad name'` at the latter two —
    /// while `CREATE TABLE t (a BIGINT DEFAULT nextval('bad name');`, whose unterminated
    /// element is the table's own column list, did **not**: it is `42602` here on both
    /// builds against 18.6's `42601`, a member of the recorded later-syntax-error class
    /// instead. The well-formed `DEFAULT nextval('bad name')` is `42602` on **both
    /// engines** throughout, which is what says the move is the reduction's *position* and
    /// not the reduction. All measured on 18.6, on the build before the quantifier gate, and here.
    ///
    /// No grammar bound is stated at this route, unlike the other two: an argument list
    /// is not optional here, so `DEFAULT nextval(ALL)` / `nextval(DISTINCT)` and their
    /// `*` spellings are `42601` from the argument parse itself, ahead of any bound and
    /// agreeing with 18.6 (measured, all four, both engines).
    ///
    /// **Both of the passes 18.6 runs ahead of the modifier are missing here**, named
    /// rather than counted, and only one of them is missing by choice.
    ///
    /// The route resolves no **signature** — it reads exactly one argument — so
    /// `DEFAULT nextval(DISTINCT 'r4s', 1)` is
    /// `42883 function nextval(unknown, integer) does not exist` on 18.6 against
    /// `42601 expected ')' found ','` here. Pre-existing rather than this rule's: the
    /// unquantified `DEFAULT nextval('r4s', 1)` and the `ALL` spelling diverge
    /// identically, `DEFAULT nextval()` is `42883` there against `42601` here, and every
    /// parenthesized twin is `42883` on **both**, since that spelling leaves the
    /// classifier for `build_call`'s arity check.
    ///
    /// And it runs no **argument transform**, which it *cannot*: the cast type is read
    /// as a bare name and thrown away because pglet models no `regclass`, and pg_dump
    /// writes a serial column's default as `nextval('x'::regclass)` — the general
    /// expression path answers that spelling
    /// `0A000 not implemented: nextval() requires a string-literal sequence name`
    /// (`parse_cast_target` lowers the cast to a `StringFn` the sequence-name
    /// look-through does not match), so coercing the cast here would refuse the dumps
    /// this route exists to load (measured:
    /// `CREATE TABLE t (a INT DEFAULT nextval('s'::regclass))`
    /// is accepted on both engines through this route, and
    /// `SELECT nextval('s'::regclass)` is that `0A000` here against `1` there). So
    /// `DEFAULT nextval(DISTINCT 's'::int)` is `42809` here against `22P02` there, its
    /// unquantified twin being this route's pre-existing and *silent* gap (accepted here
    /// against that same `22P02`) — and its **`ALL`** spelling a member
    /// reading the quantifier here *created*, silent like the free-name `ALL` accept:
    /// `DEFAULT nextval(ALL 'x'::int)` was `42601` before the quantifier was read, is
    /// `22P02` on 18.6, and is **accepted** here, minting `x`, so a following
    /// `SELECT nextval('x')` answers `1` where 18.6 is `42P01`. Measured at all three
    /// `DEFAULT` positions; the parenthesized twin is `22P02` here, agreeing, exactly as it
    /// already did unquantified. The repair is not this route's refusal to transform but a
    /// narrowing of it — coerce the discarded literal when the **written** cast type is one
    /// pglet models, leaving `regclass` alone for the reason above — which closes the
    /// unquantified twin with it; queued, and recorded in
    /// `EDGES.md`. Every other route reaches a
    /// constant-cast check: `planner::transform_call_args` for a call carrying a set
    /// quantifier (it is gated on one, for a reason of its own), and — for this family at
    /// any spelling that reaches the planner, quantified or not —
    /// `planner::sequence_call`'s own, which exists because this same name-argument
    /// reduction discards the expression there too. So the *parenthesized*
    /// `DEFAULT (nextval('s'::int))` is `22P02` here while the bare spelling this route
    /// owns is accepted.
    ///
    /// A third divergence rides that same missing transform and is **not** this route's
    /// to close, because no route here can: `DEFAULT nextval(DISTINCT c)` is
    /// `0A000 cannot use column reference in DEFAULT expression` on 18.6 against
    /// `42809` here, and so is `DEFAULT upper(DISTINCT z)` through the *general* parser,
    /// at all three `DEFAULT` positions and both spellings apiece. It is the *same*
    /// argument transform as the constant cast rather than a second mechanism —
    /// `transformColumnRef` (`parse_expr.c`) refuses a `ColumnRef` whose
    /// `ParseExprKind` is `EXPR_KIND_COLUMN_DEFAULT`, reached from
    /// `transformExprRecurse` over the arguments and so ahead of `ParseFuncOrColumn` —
    /// which is why the two halves arrive together and only one of them is pglet's to
    /// reproduce at this phase: its equivalent context key is the executor's
    /// `check_default_expr_nodes`, a phase *below* the parser this verdict is raised in,
    /// so it is the parse-time-precedence class rather than something either route can
    /// reorder. What the `42809` replaced differs by route and it is worth not
    /// flattening: through the general parser it replaced 18.6's own `0A000`
    /// byte-identically, while at **this** route the bare spelling was `42601` and the
    /// parenthesized one pglet's own
    /// `0A000 not implemented: nextval() requires a string-literal sequence name` — the
    /// same SQLSTATE as 18.6's by coincidence and a different error, which is also the
    /// unquantified control here (`DEFAULT nextval(c)` answers it on this side and
    /// 18.6's column-reference refusal on the other). All three are loud and all three
    /// are recorded in `EDGES.md`.
    fn parse_nextval_arg(&mut self) -> Result<Option<String>> {
        self.ts.expect(&TokenKind::LParen)?;
        let quantifier = self.parse_func_set_quantifier()?;
        let literal = if matches!(self.ts.peek_kind(), TokenKind::String) {
            let tok = self.ts.advance()?;
            Some(self.ts.text(&tok).to_string())
        } else {
            self.parse_expr_bp(0)?; // consume a non-string arg
            None
        };
        if self.ts.eat(&TokenKind::Cast)? {
            self.expect_name("a cast type")?; // `regclass`
        }
        self.ts.expect(&TokenKind::RParen)?;
        quantifier.refuse_distinct("nextval")?;
        // Read through the one name-literal grammar (`SplitIdentifierString`), so a
        // quoted part is unquoted and a schema qualifier (`public.s` → `s`) is stripped —
        // the literal then keys the same `Schema.sequences` slot `CREATE SEQUENCE
        // public.s` registers (which reduces via `parse_relation_name`). pg_dump emits both
        // the qualifier and, for a name needing it, the inner quoting; without this the
        // seeded `START WITH` and the resolved counter key diverge, and a quoted name
        // registers a *different* sequence whose name carries the quote characters.
        literal
            .map(|text| super::reduce_name_literal(&text))
            .transpose()
    }

    /// Parse an optional parenthesized identifier list, returning an empty `Vec` if
    /// no `(` follows.
    fn parse_optional_paren_name_list(&mut self) -> Result<Vec<String>> {
        if self.ts.at(&TokenKind::LParen) {
            self.parse_paren_name_list()
        } else {
            Ok(Vec::new())
        }
    }

    /// Parse a trailing `[CASCADE | RESTRICT]` drop-behavior clause, returning whether
    /// `CASCADE` was given.
    ///
    /// `RESTRICT` is SQL's default, so it and a bare drop both return `false` — the
    /// two are indistinguishable downstream, matching Postgres (`DROP_RESTRICT`).
    fn parse_drop_behavior(&mut self) -> Result<bool> {
        if self.ts.eat_kw(Keyword::Cascade)? {
            Ok(true)
        } else {
            let _ = self.ts.eat_kw(Keyword::Restrict)?;
            Ok(false)
        }
    }

    /// Parse a `DROP {TABLE | INDEX | TYPE | SEQUENCE} [IF EXISTS] name [, …]
    /// [CASCADE | RESTRICT]`.
    ///
    /// The trailing `CASCADE`/`RESTRICT` is load-bearing for `DROP TABLE` (a dependent
    /// foreign key or a second table's `nextval` default blocks the drop) and for
    /// `DROP SEQUENCE` (a column default blocks it) and `DROP TYPE` (a `CHECK` or
    /// `DEFAULT` converting to the enum blocks it), and inert for `INDEX`, which models no
    /// dependents.
    ///
    /// TABLE / INDEX / SEQUENCE carry the whole written name list to the executor,
    /// which resolves the names positionally before deleting any of them. **`TYPE` is
    /// deliberately still single-name** (`0A000` for a list): pglet's single-target
    /// `DROP TYPE` diverges from 18.6 twice already — a missing type is `22000` here
    /// against `42704` there, and an *in-use* type is accepted here against `2BP01`
    /// there, after which the column is **unwritable**: an `INSERT` is a loud
    /// `enum type "<n>" not found in catalog`, while existing rows still read back and
    /// concatenate. (An earlier note here said the column "silently reads as `text`";
    /// measured, only `information_schema` reports `text` — the write path is loud, which
    /// makes the consequence worse than silent, not milder.) So a multi-name
    /// fixture would pin the divergence rather than the feature.
    pub(super) fn parse_drop(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Drop)?;

        // DROP SCHEMA is shaped differently from the TABLE/INDEX/TYPE family — a
        // single schema name, and `CASCADE` is load-bearing (the reset idiom) — so
        // handle it first. `SCHEMA` is a contextual identifier (not reserved).
        if self.eat_ident("schema")? {
            let if_exists = self.parse_if_exists()?;
            let name = self.parse_relation_name()?;
            let cascade = self.parse_drop_behavior()?;
            return Ok(Plan::DropSchema {
                name,
                if_exists,
                cascade,
            });
        }

        if self.eat_ident("extension")? {
            return self.parse_drop_extension();
        }

        // The object kind: TABLE / INDEX / SEQUENCE (keywords) or TYPE (contextual
        // identifier).
        let kind = if self.ts.eat_kw(Keyword::Table)? {
            DropKind::Table
        } else if self.ts.eat_kw(Keyword::Index)? {
            DropKind::Index
        } else if self.ts.eat_kw(Keyword::Sequence)? {
            DropKind::Sequence
        } else if self.eat_ident("type")? {
            DropKind::Type
        } else {
            return Err(Error::NotImplemented(
                "only DROP TABLE / INDEX / SEQUENCE / TYPE / EXTENSION are supported".into(),
            ));
        };

        // `DROP INDEX CONCURRENTLY` — before `IF EXISTS`, as `gram.y`'s
        // `DropStmt: DROP INDEX opt_concurrently opt_if_exists any_name_list …`
        // orders them (18.6: `DROP INDEX IF EXISTS CONCURRENTLY i` is `42601`).
        // Only `INDEX` takes it; every other object kind leaves it false and so
        // still refuses the word.
        let concurrently = matches!(kind, DropKind::Index) && self.eat_ident("concurrently")?;
        let if_exists = self.parse_if_exists()?;

        let names = self.comma_list(Self::parse_object_name)?;
        let cascade = self.parse_drop_behavior()?;

        match kind {
            DropKind::Table => Ok(Plan::DropTable {
                names,
                if_exists,
                cascade,
            }),
            DropKind::Index => Ok(Plan::DropIndex {
                names,
                if_exists,
                concurrently,
            }),
            DropKind::Type => {
                let [name] = names.as_slice() else {
                    return Err(Error::NotImplemented(
                        "DROP multiple types not supported".into(),
                    ));
                };
                Ok(Plan::DropType {
                    name: name.clone(),
                    if_exists,
                    cascade,
                })
            }
            DropKind::Sequence => Ok(Plan::DropSequence {
                names,
                if_exists,
                cascade,
            }),
        }
    }

    /// Parse `TRUNCATE [TABLE] name [, …] [RESTART IDENTITY | CONTINUE IDENTITY]
    /// [CASCADE | RESTRICT]` into a [`Plan::Truncate`].
    ///
    /// `RESTART`/`CONTINUE`/`IDENTITY` are contextual identifiers. The
    /// identity / cascade tail clauses are accepted in either order (Postgres fixes
    /// the order, but accepting both is harmless and simpler).
    pub(super) fn parse_truncate(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Truncate)?;
        self.ts.eat_kw(Keyword::Table)?; // optional `TABLE` noise word

        let tables = self.comma_list(Self::parse_relation_name)?;

        let mut cascade = false;
        let mut restart_identity = false;
        loop {
            if self.eat_ident("restart")? {
                self.expect_ident("identity")?;
                restart_identity = true;
            } else if self.eat_ident("continue")? {
                self.expect_ident("identity")?; // CONTINUE IDENTITY — the default
            } else if self.ts.eat_kw(Keyword::Cascade)? {
                cascade = true;
            } else if self.ts.eat_kw(Keyword::Restrict)? {
                // RESTRICT is the default; consumed and ignored.
            } else {
                break;
            }
        }

        Ok(Plan::Truncate {
            tables,
            cascade,
            restart_identity,
        })
    }

    /// Parse `COPY <table> [ ( col, … ) ] ( FROM STDIN | TO STDOUT ) [ options ]`
    /// into a [`Plan::Copy`].
    ///
    /// Both option syntaxes are accepted: the modern parenthesized `[WITH] ( name
    /// [arg], … )` and the legacy space-separated `[WITH] DELIMITER '|' NULL 'x' CSV
    /// HEADER`. For the text format the honored options are `FORMAT`, `DELIMITER`,
    /// `NULL`, and `HEADER`; `FORMAT csv`/`binary` (and bare `CSV`/`BINARY`) parse
    /// into the plan but fail loud at execution (`0A000`) — never silently treated as
    /// text. A multi-byte or newline `DELIMITER`, a file/`PROGRAM` source/target, and
    /// an unknown option name each fail loud here.
    pub(super) fn parse_copy(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Copy)?;
        let table = self.parse_relation_name()?;

        // Optional explicit column list; empty = all columns in catalog order.
        let columns = if self.ts.at(&TokenKind::LParen) {
            self.parse_paren_name_list()?
        } else {
            Vec::new()
        };

        let direction = if self.ts.eat_kw(Keyword::From)? {
            if !self.eat_ident("stdin")? {
                return Err(Error::NotImplemented(
                    "COPY FROM supports only STDIN (a file path or PROGRAM source is unsupported)"
                        .into(),
                ));
            }
            CopyDirection::From
        } else if self.ts.eat_kw(Keyword::To)? {
            if !self.eat_ident("stdout")? {
                return Err(Error::NotImplemented(
                    "COPY TO supports only STDOUT (a file path or PROGRAM target is unsupported)"
                        .into(),
                ));
            }
            CopyDirection::To
        } else {
            return Err(self.ts.expected("FROM or TO"));
        };

        let mut opts = CopyOptionsBuilder::default();
        self.parse_copy_options(&mut opts)?;

        Ok(Plan::Copy {
            direction,
            options: CopyOptions {
                table,
                columns,
                format: opts.format,
                delimiter: opts.delimiter,
                null_marker: opts.null_marker,
                header: opts.header,
            },
        })
    }

    /// Parse the optional COPY option tail in either the parenthesized or the legacy
    /// space-separated form (a leading `WITH` is accepted, and required by the legacy
    /// form in Postgres but tolerated here in both).
    fn parse_copy_options(&mut self, opts: &mut CopyOptionsBuilder) -> Result<()> {
        self.ts.eat_kw(Keyword::With)?; // optional `WITH` noise word
        if self.ts.at(&TokenKind::LParen) {
            self.ts.bump()?;
            self.comma_list(|p| p.parse_copy_option(opts))?;
            self.ts.expect(&TokenKind::RParen)?;
        } else {
            // Legacy form: consume options up to the statement boundary. A no-option
            // `COPY … TO STDOUT` lands here already at `;`/eof, so the loop is a no-op.
            while !self.ts.at_eof() && !self.ts.at(&TokenKind::Semicolon) {
                self.parse_copy_legacy_option(opts)?;
            }
        }
        Ok(())
    }

    /// Parse one parenthesized-form COPY option `name [arg]`.
    fn parse_copy_option(&mut self, opts: &mut CopyOptionsBuilder) -> Result<()> {
        // `NULL` lexes as a keyword, so it can't come back from `expect_name`.
        if self.ts.eat_kw(Keyword::Null)? {
            opts.null_marker = self.expect_string_literal("a COPY NULL string")?;
            return Ok(());
        }
        // A COPY option name and its `FORMAT` argument are both `ColLabel`s, so
        // `expect_name` downcased an unquoted spelling and kept a quoted one exact:
        // `FORMAT "TEXT"` names no format on either engine.
        let name = self.expect_name("a COPY option name")?;
        match name.as_str() {
            "format" => {
                let fmt = self.expect_name("a COPY format")?;
                opts.format = parse_copy_format(&fmt).ok_or_else(|| {
                    Error::syntax(format!("COPY format \"{fmt}\" not recognized"))
                })?;
            }
            "delimiter" => {
                let s = self.expect_string_literal("a COPY delimiter string")?;
                opts.delimiter = parse_delimiter_byte(&s)?;
            }
            "null" => opts.null_marker = self.expect_string_literal("a COPY NULL string")?,
            "header" => opts.header = self.parse_copy_header_arg()?,
            other => {
                return Err(Error::syntax(format!("option \"{other}\" not recognized")));
            }
        }
        Ok(())
    }

    /// Parse one legacy-form COPY option (`DELIMITER [AS] '|'`, `NULL [AS] 'x'`,
    /// `CSV`, `BINARY`, `HEADER`).
    fn parse_copy_legacy_option(&mut self, opts: &mut CopyOptionsBuilder) -> Result<()> {
        if self.ts.eat_kw(Keyword::Null)? {
            self.ts.eat_kw(Keyword::As)?; // optional `AS`
            opts.null_marker = self.expect_string_literal("a COPY NULL string")?;
            return Ok(());
        }
        let name = self.expect_name("a COPY option name")?;
        match name.as_str() {
            "delimiter" => {
                self.ts.eat_kw(Keyword::As)?; // optional `AS`
                let s = self.expect_string_literal("a COPY delimiter string")?;
                opts.delimiter = parse_delimiter_byte(&s)?;
            }
            "csv" => opts.format = CopyFormat::Csv,
            "binary" => opts.format = CopyFormat::Binary,
            "header" => opts.header = true,
            other => {
                return Err(Error::syntax(format!("option \"{other}\" not recognized")));
            }
        }
        Ok(())
    }

    /// Parse a parenthesized `HEADER` boolean argument; a bare `HEADER` (next token a
    /// `,` or `)`) is `true`.
    ///
    /// The word is folded **unconditionally**, quoted spellings included, because this
    /// is a *value* position and not a name one: Postgres reads the option's argument
    /// as a string and compares it with `pg_strcasecmp` (`copy.c`'s HEADER-option
    /// reader), so a quoted spelling folds there too — `HEADER On`, `HEADER "ON"` and
    /// `HEADER "TRUE"` are each accepted on 18.6.
    ///
    /// The *set* of accepted words is a different question from the folding, and the
    /// two engines answer it differently in both directions. Measured on 18.6 over
    /// `COPY … TO STDOUT WITH (HEADER …)`: `"TRUE"` emits the header there and is
    /// `42601` here (a quoted word is not the `TRUE` keyword, and `true` is not in the
    /// match below), while `"t"`, `"yes"`, `"1"` and `"0"` are each
    /// `header requires a Boolean value or "match"` there and accepted here. `"ON"`
    /// agrees. (The 20devel PostgreSQL source's reader additionally takes an
    /// integer line count; the 18.6 oracle the fixtures assert against does not, so
    /// that is one to read from the instance rather than the source.)
    fn parse_copy_header_arg(&mut self) -> Result<bool> {
        if self.ts.at(&TokenKind::Comma) || self.ts.at(&TokenKind::RParen) {
            return Ok(true);
        }
        if self.ts.eat_kw(Keyword::True)? {
            return Ok(true);
        }
        if self.ts.eat_kw(Keyword::False)? {
            return Ok(false);
        }
        let tok = self.ts.advance()?;
        let text = self.ts.text(&tok).to_string();
        match text.to_ascii_lowercase().as_str() {
            "on" | "t" | "1" | "yes" => Ok(true),
            "off" | "f" | "0" | "no" => Ok(false),
            _ => Err(Error::syntax(format!(
                "invalid value for boolean option \"header\": {text}"
            ))),
        }
    }

    /// Parse a `BEGIN` / `START TRANSACTION` into [`Plan::BeginTransaction`].
    ///
    /// The optional `TRANSACTION`/`WORK` noise word is consumed, then the
    /// transaction-mode list — see [`Self::parse_transaction_modes`] for what each
    /// mode does here.
    pub(super) fn parse_begin(&mut self) -> Result<Plan> {
        if self.ts.eat_kw(Keyword::Start)? {
            // `START TRANSACTION` requires the keyword.
            self.ts.expect_kw(Keyword::Transaction)?;
        } else {
            self.ts.expect_kw(Keyword::Begin)?;
            // `BEGIN [TRANSACTION | WORK]`.
            if !self.ts.eat_kw(Keyword::Transaction)? {
                self.eat_ident("work")?;
            }
        }
        let isolation = self.parse_transaction_modes()?;
        Ok(Plan::BeginTransaction { isolation })
    }

    /// Parse `gram.y`'s `transaction_mode_list` — the optional tail of `BEGIN` and
    /// `START TRANSACTION`.
    ///
    /// The modes are `ISOLATION LEVEL {SERIALIZABLE | REPEATABLE READ | READ
    /// COMMITTED | READ UNCOMMITTED}`, `READ WRITE` / `READ ONLY`, and `[NOT]
    /// DEFERRABLE`, in **any order and combination**, with the comma between them
    /// optional (`transaction_mode_list` has both a comma'd and a juxtaposed
    /// production). A `SET TRANSACTION …` writes the same list and rides the `SET`
    /// seam instead.
    ///
    /// One is carried, one is accepted and inert, and one is refused:
    ///
    /// - **`ISOLATION LEVEL …`** is carried out as the block's level — the last one
    ///   written, as 18.6 applies the list in order — and the host acts on it: a
    ///   `REPEATABLE READ` or `SERIALIZABLE` block keeps the snapshot it began with and
    ///   is refused `40001` at its first write when anything committed since
    ///   (`pglet::TransactionSession::take_writer`), which is coarser than 18.6's
    ///   per-row check and never answers the lost update 18.6 refuses.
    ///   `docs/storage.md` § Isolation Guarantees is the statement.
    /// - **`[NOT] DEFERRABLE`** is parsed and ignored. It only has an effect in
    ///   combination with `SERIALIZABLE READ ONLY`, which is refused below.
    /// - **`READ ONLY`** is refused (`0A000`). It is the one mode that would be a
    ///   *silent over-acceptance*: pglet has no read-only block, so accepting the
    ///   request and then executing the write it forbids answers success where
    ///   18.6 answers `25006`. Enforcing it is the better answer and is tracked as
    ///   its own work; refusing is the answer that cannot write. `READ WRITE` — the
    ///   default, and what a pool that spells the mode out usually sends — is
    ///   accepted.
    ///
    /// # Errors
    ///
    /// `42601` for a malformed mode or a trailing comma, `0A000` for `READ ONLY`.
    fn parse_transaction_modes(&mut self) -> Result<Option<IsolationLevel>> {
        let mut isolation = None;
        if !self.at_transaction_mode() {
            return Ok(isolation);
        }
        loop {
            if let Some(level) = self.parse_transaction_mode()? {
                isolation = Some(level);
            }
            let comma = self.ts.eat(&TokenKind::Comma)?;
            if !self.at_transaction_mode() {
                return if comma {
                    // A comma promises another mode; `BEGIN READ WRITE,` is `42601`
                    // on 18.6 and must not be swallowed into an accepted `BEGIN`.
                    Err(self.ts.syntax_error_at_or_near())
                } else {
                    Ok(isolation)
                };
            }
        }
    }

    /// Whether the next token can begin a `transaction_mode_item`.
    ///
    /// `NOT` is the only keyword among the four leading words; the rest are
    /// unreserved and reach the parser as identifiers. A `NOT` that is not followed
    /// by `DEFERRABLE` is a syntax error raised by [`Self::parse_transaction_mode`],
    /// not a reason to stop reading modes — nothing else in the grammar's tail can
    /// begin with it.
    fn at_transaction_mode(&self) -> bool {
        self.at_ident("isolation")
            || self.at_ident("read")
            || self.at_ident("deferrable")
            || self.ts.at_kw(Keyword::Not)
    }

    /// One `transaction_mode_item`, answering the isolation level it names, if it
    /// names one. See [`Self::parse_transaction_modes`].
    fn parse_transaction_mode(&mut self) -> Result<Option<IsolationLevel>> {
        if self.eat_ident("isolation")? {
            self.expect_ident("level")?;
            return self.parse_isolation_level().map(Some);
        }
        if self.eat_ident("read")? {
            if self.eat_ident("only")? {
                return Err(Error::NotImplemented(
                    "READ ONLY transactions are not supported".into(),
                ));
            }
            self.expect_ident("write")?;
            return Ok(None);
        }
        if self.eat_ident("deferrable")? {
            return Ok(None);
        }
        self.ts.expect_kw(Keyword::Not)?;
        self.expect_ident("deferrable")?;
        Ok(None)
    }

    /// `gram.y`'s `iso_level`.
    fn parse_isolation_level(&mut self) -> Result<IsolationLevel> {
        if self.eat_ident("serializable")? {
            return Ok(IsolationLevel::Serializable);
        }
        if self.eat_ident("repeatable")? {
            self.expect_ident("read")?;
            return Ok(IsolationLevel::RepeatableRead);
        }
        self.expect_ident("read")?;
        if self.eat_ident("committed")? {
            return Ok(IsolationLevel::ReadCommitted);
        }
        self.expect_ident("uncommitted")?;
        Ok(IsolationLevel::ReadUncommitted)
    }

    /// Parse a `COMMIT [TRANSACTION | WORK]` into [`Plan::CommitTransaction`].
    pub(super) fn parse_commit(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Commit)?;
        if !self.ts.eat_kw(Keyword::Transaction)? {
            self.eat_ident("work")?;
        }
        Ok(Plan::CommitTransaction)
    }

    /// Parse a `ROLLBACK [TRANSACTION | WORK] [TO [SAVEPOINT] <name>]` into
    /// [`Plan::RollbackTransaction`] or [`Plan::RollbackToSavepoint`].
    ///
    /// The `TO` tail sits **after** the noise word, matching `gram.y`'s
    /// `ROLLBACK opt_transaction TO SAVEPOINT ColId` — so `ROLLBACK WORK TO
    /// SAVEPOINT s` and `ROLLBACK TRANSACTION TO s` both parse, measured on 18.6.
    pub(super) fn parse_rollback(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Rollback)?;
        if !self.ts.eat_kw(Keyword::Transaction)? {
            self.eat_ident("work")?;
        }
        if self.ts.eat_kw(Keyword::To)? {
            return Ok(Plan::RollbackToSavepoint {
                name: self.parse_savepoint_name()?,
            });
        }
        Ok(Plan::RollbackTransaction)
    }

    /// Parse a `SAVEPOINT <name>` into [`Plan::Savepoint`].
    pub(super) fn parse_savepoint(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Savepoint)?;
        Ok(Plan::Savepoint {
            name: self.expect_name("a savepoint name")?,
        })
    }

    /// Parse a `RELEASE [SAVEPOINT] <name>` into [`Plan::ReleaseSavepoint`].
    ///
    /// There is no `RELEASE TRANSACTION` spelling — `gram.y` gives `RELEASE` only
    /// the optional `SAVEPOINT` noise word, so `RELEASE TRANSACTION s` is `42601` on
    /// 18.6 and here.
    pub(super) fn parse_release(&mut self) -> Result<Plan> {
        self.ts.expect_kw(Keyword::Release)?;
        Ok(Plan::ReleaseSavepoint {
            name: self.parse_savepoint_name()?,
        })
    }

    /// Parse `[SAVEPOINT] <name>` — the shared tail of `ROLLBACK TO` and `RELEASE`.
    ///
    /// **The noise word is itself a legal savepoint name**, because `SAVEPOINT` is
    /// unreserved and a savepoint name is a `ColId`. `gram.y` separates the two
    /// spellings with a whole extra production (`RELEASE SAVEPOINT ColId` beside
    /// `RELEASE ColId`); one token of lookahead does the same job here — a
    /// `SAVEPOINT` token followed by another name token is the noise word,
    /// otherwise it is the name. Both `SAVEPOINT savepoint; RELEASE savepoint` and
    /// `RELEASE SAVEPOINT savepoint` are accepted on 18.6, measured.
    fn parse_savepoint_name(&mut self) -> Result<String> {
        if self.ts.at_kw(Keyword::Savepoint) && super::is_colid_name(self.ts.peek2_kind()) {
            self.ts.bump()?;
        }
        self.expect_name("a savepoint name")
    }

    /// Parse an optional `IF EXISTS` clause; returns whether it was present.
    pub(super) fn parse_if_exists(&mut self) -> Result<bool> {
        if self.ts.eat_kw(Keyword::If)? {
            self.ts.expect_kw(Keyword::Exists)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Parse an optional `IF NOT EXISTS` clause; returns whether it was present.
    pub(super) fn parse_if_not_exists(&mut self) -> Result<bool> {
        if self.ts.eat_kw(Keyword::If)? {
            self.ts.expect_kw(Keyword::Not)?;
            self.ts.expect_kw(Keyword::Exists)?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Consume a string-literal token, returning its decoded value, or a `42601`
    /// error naming `what`.
    pub(super) fn expect_string_literal(&mut self, what: &str) -> Result<String> {
        let tok = self.ts.advance()?;
        if matches!(tok.kind, TokenKind::String) {
            Ok(self.ts.text(&tok).to_string())
        } else {
            Err(self.ts.expected(what))
        }
    }
}

/// Lower a parsed table constraint from `ALTER TABLE … ADD …` into its plan.
///
/// `CHECK` keeps the dedicated [`Plan::AlterTableAddCheck`] arm; `PRIMARY KEY` /
/// `UNIQUE` / `FOREIGN KEY` (single- or multi-column) lower to
/// [`Plan::AlterTableAddConstraint`] (real enforcement).
fn lower_alter_add_constraint(
    table: String,
    table_if_exists: bool,
    parsed: ParsedTableConstraint,
) -> Plan {
    let constraint = match parsed {
        ParsedTableConstraint::Check(check) => {
            return Plan::AlterTableAddCheck {
                table,
                check,
                table_if_exists,
            };
        }
        ParsedTableConstraint::PrimaryKey { name, columns } => {
            AddConstraint::PrimaryKey { columns, name }
        }
        ParsedTableConstraint::Unique { name, columns } => AddConstraint::Unique { columns, name },
        ParsedTableConstraint::ForeignKey(fk) => AddConstraint::ForeignKey(fk),
    };
    Plan::AlterTableAddConstraint {
        table,
        constraint,
        table_if_exists,
    }
}

/// The bare-column names of an index key, or `None` if any part is an expression.
///
/// `Some(names)` (one per part, in key order) only when every part is a bare
/// `Expr::Column` with no table qualifier — the `IndexKey::Columns` fast path;
/// otherwise the key folds to `IndexKey::Exprs`.
/// The extensions pglet answers `CREATE`/`DROP EXTENSION` for.
///
/// Membership means **this engine already provides what the extension provides**,
/// not that anything is installed: `pgcrypto`'s `gen_random_uuid()` and
/// `uuid-ossp`'s `uuid_generate_v4()` are ordinary builtins here, resolvable with or
/// without the statement, and `plpgsql` is what the `DO $$ … $$` block is written
/// in. A name outside this set is `0A000` — the same answer 18.6 gives for an
/// extension whose control file it cannot find.
///
/// The names are compared **exactly**, against a name the parser has already folded:
/// an unquoted `PGCRYPTO` downcases and matches, a quoted `"UUID-OSSP"` does not —
/// both as measured on 18.6, where the quoted upper-case spelling is
/// `extension "UUID-OSSP" is not available`.
const KNOWN_EXTENSIONS: [&str; 3] = ["pgcrypto", "plpgsql", "uuid-ossp"];

/// Whether a two-part relation name's qualifier is one the `ALTER TABLE IF EXISTS` skip may
/// reduce on: neither `public`, where the tables are, nor `information_schema`, nor a
/// `pg_`-prefixed name — the prefix 18.6 reserves for its system schemas (`CREATE SCHEMA
/// pg_x` is `42939` there, measured), under which it holds catalog relations pglet does
/// not model. pglet
/// creates no other schema (`CREATE SCHEMA` is `0A000` for every name but `public`), so
/// 18.6 finds no relation under such a qualifier in any database pglet's own statements
/// build, and `ALTER TABLE IF EXISTS` over it is the skip there (measured, `"Public"`
/// included). An `information_schema` or `pg_`-prefixed qualifier keeps the reduction's
/// `42P01`: under a system schema, 18.6's answer turns on catalog relations pglet has no
/// list of (`pg_catalog.pg_class` is `42501`, `pg_catalog.<a missing name>` the skip), and
/// a `pg_`-prefixed qualifier naming no schema (`pg_nosuch`) is 18.6's skip as well.
fn qualifier_holds_no_relation(parts: &[String]) -> bool {
    matches!(parts, [schema, _]
        if schema != "public" && schema != "information_schema" && !schema.starts_with("pg_"))
}

/// Whether `name` is one of [`KNOWN_EXTENSIONS`].
fn is_known_extension(name: &str) -> bool {
    KNOWN_EXTENSIONS.contains(&name)
}

/// [`is_known_extension`] as a gate, raising 18.6's `0A000` for a name outside the
/// set.
fn require_known_extension(name: &str) -> Result<()> {
    if is_known_extension(name) {
        Ok(())
    } else {
        Err(Error::ExtensionNotAvailable(name.to_string()))
    }
}

fn bare_column_names(parts: &[Expr]) -> Option<Vec<String>> {
    parts
        .iter()
        .map(|part| match part {
            Expr::Column { table: None, name } => Some(name.clone()),
            _ => None,
        })
        .collect()
}

/// The accumulated COPY options, seeded with the text-format defaults (tab
/// delimiter, `\N` null marker, no header). The parser overwrites fields as it reads
/// the option list.
struct CopyOptionsBuilder {
    format: CopyFormat,
    delimiter: u8,
    null_marker: String,
    header: bool,
}

impl Default for CopyOptionsBuilder {
    fn default() -> Self {
        Self {
            format: CopyFormat::Text,
            delimiter: b'\t',
            null_marker: "\\N".to_string(),
            header: false,
        }
    }
}

/// Map a `FORMAT` option value to a [`CopyFormat`], or `None` for an unrecognized
/// format name.
///
/// `name` arrives from [`Parser::expect_name`], already downcased if the source
/// wrote it unquoted, so the comparison is exact — `FORMAT "TEXT"` is not `text`
/// (18.6: `22023 COPY format "TEXT" not recognized`; pglet raises its own
/// `42601`-shaped "not recognized", the SQLSTATE it already gives an unrecognized
/// bare format).
fn parse_copy_format(name: &str) -> Option<CopyFormat> {
    match name {
        "text" => Some(CopyFormat::Text),
        "csv" => Some(CopyFormat::Csv),
        "binary" => Some(CopyFormat::Binary),
        _ => None,
    }
}

/// Validate a `DELIMITER` string as a single one-byte character, returning that byte.
///
/// Fails loud (`0A000`) on a multi-byte delimiter or a `\r`/`\n` delimiter, matching
/// Postgres's "COPY delimiter must be a single one-byte character" / "cannot be
/// newline or carriage return".
fn parse_delimiter_byte(s: &str) -> Result<u8> {
    let bytes = s.as_bytes();
    let [b] = bytes else {
        return Err(Error::NotImplemented(
            "COPY delimiter must be a single one-byte character".into(),
        ));
    };
    if *b == b'\r' || *b == b'\n' {
        return Err(Error::NotImplemented(
            "COPY delimiter cannot be newline or carriage return".into(),
        ));
    }
    Ok(*b)
}

#[cfg(test)]
mod windowing_tests {
    use crate::catalog::Schema;
    use crate::parser::parse;

    fn state(sql: &str) -> Option<&'static str> {
        parse(sql, &Schema::default()).err().map(|e| e.sqlstate())
    }

    /// A window function is forbidden in every **persisted** `Expr` context — a CHECK
    /// constraint (column-level and table-level), an index predicate, and an index
    /// expression — matching Postgres's `42P20`. This is load-bearing for
    /// persistence: it keeps an `Expr::Window` (whose `order_by` field is
    /// postcard-serialized) out of the catalog, so widening that field needs no
    /// store-magic bump.
    #[test]
    fn window_in_persisted_expr_context_is_windowing_error() {
        // Column-level and table-level CHECK.
        assert_eq!(
            state("CREATE TABLE t (a int CHECK (row_number() OVER () > 0))"),
            Some("42P20")
        );
        assert_eq!(
            state("CREATE TABLE t (a int, CHECK (rank() OVER (ORDER BY a) > 0))"),
            Some("42P20")
        );
        // ALTER TABLE ADD CONSTRAINT ... CHECK (shares the table-constraint parser).
        assert_eq!(
            state("ALTER TABLE t ADD CONSTRAINT c CHECK (row_number() OVER () > 0)"),
            Some("42P20")
        );
        // Partial-index predicate and index expression.
        assert_eq!(
            state("CREATE INDEX ix ON t (a) WHERE (row_number() OVER () > 0)"),
            Some("42P20")
        );
        assert_eq!(
            state("CREATE INDEX ix ON t ((row_number() OVER ()))"),
            Some("42P20")
        );
    }

    /// A window nested inside a larger CHECK expression is still caught (the walk is
    /// not just top-level).
    #[test]
    fn nested_window_in_check_is_windowing_error() {
        assert_eq!(
            state("CREATE TABLE t (a int CHECK (a + (row_number() OVER ()) > 0))"),
            Some("42P20")
        );
    }

    /// A subquery is forbidden in every persisted `Expr` context — a CHECK
    /// constraint (column-level, table-level, `ALTER TABLE ADD CONSTRAINT`), an index
    /// predicate, and an index expression — matching Postgres's `0A000` (`cannot use
    /// subquery in …`). This is what makes the persistence invariant airtight: a
    /// subquery is the only remaining way a window (or any plan-bearing `Expr`) could
    /// reach a persisted tree.
    #[test]
    fn subquery_in_persisted_expr_context_is_feature_not_supported() {
        assert_eq!(
            state("CREATE TABLE t (a int CHECK (a > (SELECT 1)))"),
            Some("0A000")
        );
        assert_eq!(
            state("CREATE TABLE t (a int, CHECK (a > (SELECT 1)))"),
            Some("0A000")
        );
        assert_eq!(
            state("ALTER TABLE t ADD CONSTRAINT c CHECK (a > (SELECT 1))"),
            Some("0A000")
        );
        assert_eq!(
            state("CREATE INDEX ix ON t (a) WHERE (a > (SELECT 1))"),
            Some("0A000")
        );
        assert_eq!(
            state("CREATE INDEX ix ON t ((a + (SELECT 1)))"),
            Some("0A000")
        );
        // `EXISTS` and `IN (SELECT …)` are subqueries too.
        assert_eq!(
            state("CREATE TABLE t (a int CHECK (EXISTS (SELECT 1)))"),
            Some("0A000")
        );
        assert_eq!(
            state("CREATE TABLE t (a int CHECK (a IN (SELECT 1)))"),
            Some("0A000")
        );
    }

    /// A window reachable only *inside* a subquery is rejected as the **subquery**
    /// (`0A000`), matching Postgres — which never descends to the window. This closes
    /// the persistence hole: the CREATE is rejected up front (not silently accepted,
    /// then failing at INSERT), so no `Expr::Window` is ever persisted.
    #[test]
    fn window_via_subquery_in_check_is_subquery_error_not_windowing() {
        assert_eq!(
            state("CREATE TABLE t (a int CHECK (a > (SELECT row_number() OVER ())))"),
            Some("0A000")
        );
    }

    /// An aggregate is forbidden in a persisted `Expr` context — Postgres's `42803`
    /// (`aggregate functions are not allowed in …`). Conformance, not persistence
    /// (an aggregate carries no `OrderKey`).
    #[test]
    fn aggregate_in_persisted_expr_context_is_grouping_error() {
        assert_eq!(
            state("CREATE TABLE t (a int CHECK (sum(a) > 0))"),
            Some("42803")
        );
        assert_eq!(
            state("CREATE INDEX ix ON t (a) WHERE (count(*) > 0)"),
            Some("42803")
        );
        assert_eq!(state("CREATE INDEX ix ON t ((sum(a)))"), Some("42803"));
    }

    /// An ordinary CHECK / index — no window, subquery, or aggregate — is unaffected
    /// (no over-rejection): scalar/boolean predicates, plain functions, and legit
    /// index expressions / partial-index `WHERE`s all still parse.
    #[test]
    fn ordinary_check_and_index_still_parse() {
        assert!(parse("CREATE TABLE t (a int CHECK (a > 0))", &Schema::default()).is_ok());
        assert!(
            parse(
                "CREATE TABLE t (a int CHECK (lower(a) = 'x'))",
                &Schema::default()
            )
            .is_ok()
        );
        assert!(parse("CREATE INDEX ix ON t (a) WHERE (a > 0)", &Schema::default()).is_ok());
        assert!(parse("CREATE INDEX ix ON t (lower(a))", &Schema::default()).is_ok());
        assert!(parse("CREATE INDEX ix ON t ((a + 1))", &Schema::default()).is_ok());
    }
}
