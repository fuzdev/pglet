//! DDL (Data Definition Language) execution.
//!
//! Handles CREATE TABLE, DROP TABLE, ALTER TABLE operations.

use std::collections::{BTreeMap, HashMap, HashSet};

use crate::catalog::{
    CheckConstraint, Column, ColumnDefault, ConstraintKind, EnumType, ForeignKey,
    IdentityGeneration, Index, IndexKey, Schema, Sequence, SequenceOwner, Table, Uniqueness,
    is_system_column_name, make_object_name,
};
use crate::planner::{
    AddConstraint, BoolContext, CheckConstraintDef, Expr, ForeignKeyDef, PlannerScope,
    SequenceOwnerSpec, SequenceRestart, UniqueConstraint, branch_unification_legality,
    check_boolean_operand, check_expr_node_operand_types,
};
use crate::types::Value;
use crate::{Error, RelationKind, Result, WrongTypeRoute};

use super::Storage;
use super::constraints::{
    coerce_column_value, enforce_checks, parent_tuple_exists, prepare_check,
    single_referenced_column,
};
use super::dependencies::{
    Dependents, DropTarget, ExpressionRef, default_sequence_refs, identity_sequence_owner,
};
use super::dml::{SequenceDraw, eval_default_expr, resolve_default};
use super::helpers::{
    DropResolution, drop_skip_notice, generate_object_oid, get_table_or_err, next_object_oid,
    resolve_relation_for_drop,
};
use super::type_resolution::resolve_column_type;
use super::types::{
    IndexMethod, IndexSpec, QueryResult, RowId, index_method_for, index_spec_from_catalog,
};
use crate::types::TypeOid;

/// Reject a `GENERATED … AS IDENTITY` column whose type isn't an integer. Postgres
/// backs an identity column with an implicit sequence, so it restricts the type to
/// `smallint`/`integer`/`bigint` and raises `22023` otherwise — matched here for both
/// the inline `CREATE TABLE` and the `ALTER … ADD GENERATED` paths.
const fn ensure_identity_integer(type_oid: TypeOid) -> Result<()> {
    if matches!(type_oid, TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8) {
        Ok(())
    } else {
        Err(Error::IdentityColumnType)
    }
}

/// Whether a route merely **stores** a column `DEFAULT` or also **evaluates** it as
/// part of the same statement — see [`cook_column_default`]'s `timing` paragraph.
#[derive(Clone, Copy, PartialEq, Eq)]
enum DefaultTiming {
    /// `CREATE TABLE` and `ALTER COLUMN … SET DEFAULT`: the expression is recorded and
    /// a value-dependent failure is the first write's.
    Deferred,
    /// `ALTER TABLE … ADD COLUMN`: 18.6 evaluates the default once for the column's
    /// `attmissingval`, so the value-dependent failure is this statement's — over an
    /// empty table too.
    Eager,
}

/// Cook a column `DEFAULT` at the position 18.6 cooks one — the only place a column
/// default's verdict is decided.
///
/// **Every parse-analysis verdict a column `DEFAULT` can draw is raised here.** The
/// scope is the verdict *kind*, not the whole clause: a spelling the grammar itself
/// cannot read is still the parser's (`42601` for a malformed `b_expr`, and the
/// `nextval` classifier's own refusal of a non-literal argument), exactly as bison
/// answers ahead of 18.6's parse analysis — see the engine-valued arms' carve-out
/// below. What must not
/// move into the parser is the rest, because 18.6 raises all of it in parse analysis,
/// which sits *behind* the statement-level gates a parse-time refusal jumps in front
/// of. **What outranks a bad DEFAULT is whatever 18.6 decides earlier in the same
/// statement**, so the loser list grows with every gate put ahead of the cook and is
/// an enumeration of measured members rather than a closed set. On `CREATE TABLE`,
/// measured on both engines: the column type's `42704`, the `transformIndexConstraints`
/// pass (`42P16`, a key column's `42703`, a repeated key column's `42701`), an identity
/// column's declared `SEQUENCE NAME` collision (`42P07`), the relation's column-name
/// set (a duplicate and a system-owned name, each `42701`) and the table's own
/// `42P07`; on the `ALTER` routes, the missing relation (`42P01`), the column-name
/// collision `check_for_column_name_collision` answers for both a user column and a
/// system-owned one (`42701`) and the missing column (`42703`). A plain later syntax
/// error outranks it too, bison reporting before parse analysis begins.
/// It **beats** the constraint *naming* block behind it — a duplicate CHECK name
/// (`42710`), an ill-typed CHECK (`42883`), a taken constraint-index name (`42P07`), a
/// missing FK parent (`42P01`) and a repeated column in an FK's referenced list
/// (`42830`), in either written order — and that block is not every constraint gate:
/// a `PRIMARY KEY`/`UNIQUE` key list is validated in the pass above, which is why
/// `CREATE TABLE p (a INT DEFAULT 'zz', PRIMARY KEY (a, a))` is `42701` and not
/// `22P02` on both engines. `42P16` **changes sides by route**: the `ADD COLUMN`
/// inline `PRIMARY KEY`'s multiplicity sits behind the cook there (see the `timing`
/// paragraph), so a member of one route's list is not a member of the other's by
/// inheritance. Between two
/// bad defaults the **declared column order** wins (measured both ways). Each caller
/// therefore places this call rather than inheriting a position; see the three call
/// sites' own comments.
///
/// **`timing` is the one thing the routes do not share**, and it is 18.6's own split
/// rather than a knob: `CREATE TABLE` and `ALTER COLUMN … SET DEFAULT` only *store*
/// the expression, while `ALTER TABLE … ADD COLUMN` **evaluates** it, once, for the
/// `attmissingval` the added column's existing-and-future rows read
/// (`StoreAttrMissingVal`). So the value-dependent family — `22012`, `22003`, `22001`
/// — is a DDL error on the `ADD COLUMN` route and a per-row error on the other two,
/// **even over an empty table**: measured on 18.6 over a table with no rows,
/// `ADD COLUMN a INT DEFAULT 1/0` is `22012`, `… b INT DEFAULT (2147483647 + 1)` is
/// `22003`, `… c VARCHAR(3) DEFAULT 'abcdef'` is `22001` and
/// `… d NUMERIC(4,2) DEFAULT 12345.6` is `22003`, while the identical defaults on
/// `CREATE TABLE` and `SET DEFAULT` are all accepted. Its statement position is the
/// same one the cook-time family takes, measured on the same route: it loses to the
/// missing relation (`42P01`), the column type (`42704`) and both names
/// `check_for_column_name_collision` answers — a column the table already has and a
/// system-owned one, each `42701`, the second measured against this family's own
/// `22012` (`ADD COLUMN ctid INT DEFAULT 1/0`)
/// — and beats the inline `PRIMARY KEY` multiplicity, a taken
/// constraint-index name, a duplicate inline CHECK name, an ill-typed inline CHECK
/// and a missing inline `REFERENCES` parent — so one flag at this one call decides
/// it, with nothing to reorder.
///
/// Four checks, in this order.
///
/// 1. **The constructs 18.6 forbids in a `DEFAULT`**, positionally within the
///    expression ([`check_default_expr_nodes`]).
/// 2. **A probe evaluation.** A `DEFAULT` expression is closed by construction — the
///    walk above has just refused every column reference and parameter — so unlike
///    the CHECK probe this is not a synthetic row but *the* value the default
///    resolves to. Only an [`is_cook_time_error`] outcome propagates from here; a
///    value-dependent one is 18.6's to raise per row, which it does under
///    [`DefaultTiming::Deferred`] (`DEFAULT 1/0` is accepted at DDL and `22012` at the
///    first insert, measured) and which [`DefaultTiming::Eager`] raises **after**
///    step 3.
/// 3. **The assignment cast to the column type**, which 18.6 performs while cooking
///    (`coerce_to_target_type`), and which is **two rules rather than one** — the
///    split being Postgres's `unknown`:
///    - A **typed** expression is a pure type verdict, reached without evaluating
///      anything: `coerce_to_target_type` returns NULL and `cookDefault` raises
///      `42804 column "x" is of type integer but default expression is of type text`
///      (`src/backend/catalog/heap.c` in the PostgreSQL source). That is
///      [`crate::planner::check_assignment`] here — a caller of that wrapper beside
///      three of the four DML assignment-lowering sites (re-derive the caller list by
///      grepping `check_assignment`); the fourth DML site,
///      `INSERT … SELECT`'s `check_insert_select_assignments`, reaches the rulebook
///      through [`crate::planner::assignment_legality`] directly, as this cook's own
///      bare-constant arm below does. So a `DEFAULT` and an `INSERT` of the same
///      value cannot disagree — and it must run **before** step 2's held failure,
///      because 18.6 never evaluates: `DEFAULT (1/0)::text` and
///      `DEFAULT (2147483647 + 1)::text` on an `INT` column are `42804` there on both
///      routes and were `22012` / `22003` here.
///    - An **unknown** constant instead runs the target type's *input function*, since
///      an unknown is folded into a typed constant in place — and 18.6 runs *that*
///      inside `transformExpr` when it sits under an explicit cast, so it comes first:
///      `INT DEFAULT 'zzz'::date` is `22007` on both engines, not `42804`. Step 2's
///      cook-time family is what reproduces that ordering — for a cast the probe's
///      evaluation reaches; one in a branch it does not take raises nothing, where 18.6
///      still refuses it (`EDGES.md` row 47). `DEFAULT 'x'` on `INT` is
///      `22P02` there and here, while `DEFAULT 'abcdef'` on `VARCHAR(3)` (`22001`) and
///      `DEFAULT 12345.6` on `NUMERIC(4,2)` (`22003`) are **accepted** and raise per
///      row — those typmods are applied after the cast, not by it. So the value
///      coercion is run last and [`is_cook_time_error`] decides what escapes it, again
///      only under [`DefaultTiming::Deferred`]; the two typmod codes are exactly the
///      `22001` / `22003` an `ADD COLUMN` raises eagerly.
/// 4. **A render probe**, which is pglet's own and therefore runs **last** — see the
///    call site below. **Step 3's value half can be skipped and step 4 cannot**: a
///    deferred outcome in step 2 leaves no value to coerce but still leaves an
///    expression to render, so that arm falls *through* rather than returning.
///    Returning there was a hole of exactly one statement's width and its cost was the
///    whole database's dump — see the render probe's own comment.
///
/// Steps 2–3 are skipped for the nine engine-valued closed arms (`now()`-family,
/// `CURRENT_DATE`/`TIME`, `gen_random_uuid()`, `nextval`): probing `nextval` would
/// have to either advance a live sequence or invent a value, and the other eight
/// carry no expression to render. A `DEFAULT now()` on an `INT` column is 18.6's
/// `42804` and is accepted here — a pre-existing divergence not closed here
/// (`EDGES.md`). Its old companion, `DEFAULT true` on an
/// `INT` column, **is** closed: `bool ↔ int` is an explicit-only cast in `pg_cast`, and
/// `constraints::coerce_column_value` now refuses that pair in assignment context
/// (both directions, `42804`) while leaving `SELECT true::int` / `SELECT 1::bool`
/// answering. **The code is 18.6's; the rendered sentence is not, in three measured
/// ways**, and the shorter claim this line used to carry ("18.6's own code and
/// sentence") overstated all three. `Error::Type` renders a `type error: ` prefix
/// nothing in 18.6 emits. 18.6's *default* route says `… but default expression is of
/// type …` while pglet reuses the plain assignment sentence (`… but expression is of
/// type …`) on every route. And the source type it names can differ: a pglet integer
/// literal is an `Int8`, so `DEFAULT 1` on a `BOOL` column reads `bigint` where 18.6
/// reads `integer` — same SQLSTATE, different noun. Measured on both engines for
/// `INT DEFAULT true`, `BOOL DEFAULT 1` and `BIGINT DEFAULT true`.
fn cook_column_default(
    default: &ColumnDefault,
    column: &Column,
    timing: DefaultTiming,
    schema: &Schema,
) -> Result<()> {
    // **A `nextval('x')` naming an enum and no relation is `42P01`**, 18.6's answer:
    // `'x'::regclass` is resolved as the default is cooked and finds nothing in
    // `pg_class`. For a name nothing holds pglet registers the sequence instead (the
    // documented auto-creation), which here would put a sequence and a type under one
    // name — a pair `heap_create_with_catalog` refuses, and so a dump no engine
    // restores.
    for seq in default_sequence_refs(default) {
        if schema.relation_kind(&seq).is_none() && schema.enums.contains_key(&seq) {
            return Err(Error::SequenceNotFound(seq));
        }
    }
    // **A sequence-bearing expression is never cooked eagerly**, because the probe's
    // `nextval` stand-in is not the value 18.6 would compute. `SequenceDraw::Probe`
    // substitutes `1` whatever the counter holds, so over a sequence already at 5,
    // `ADD COLUMN c INT DEFAULT 100 / (nextval('s') - 1)` divides by zero here and by
    // five there — an eager verdict about a value the statement never produces. The
    // deferred family stays deferred for those, which is the direction that only
    // under-refuses.
    let eager = timing == DefaultTiming::Eager && default_sequence_refs(default).is_empty();
    // A probe failure 18.6 raises *after* the assignment cast, held rather than
    // returned so step 3 can answer first — see that step's own comment.
    let mut held: Option<Error> = None;
    let probe = match default {
        ColumnDefault::Expr(expr) => {
            check_default_expr_nodes(expr)?;
            let probe = match eval_default_expr(expr, &mut BTreeMap::new(), SequenceDraw::Probe) {
                Ok(value) => Some(value),
                // A conversion to an enum that reaches evaluation is one of a *value* —
                // `'q'::text::mq` — which 18.6 runs per row, not in parse analysis: a
                // literal's `'q'::mq` is refused by the node walk above, so this `22P02`
                // is the value-dependent kind, held (`CREATE TABLE … DEFAULT
                // 'q'::text::mq` and `SET DEFAULT` accept it on 18.6, and the `ADD
                // COLUMN` route raises it eagerly — measured).
                Err(e)
                    if is_cook_time_error(&e) && !matches!(e, Error::InvalidEnumValue { .. }) =>
                {
                    return Err(e);
                }
                // A value-dependent failure (`1/0`, an overflow) is 18.6's to raise
                // per row, so the default is recorded and the error lands at the
                // first write, as it does there. **`None`, not an early return**:
                // there is no value to run step 3 against, and step 4 still has an
                // expression to render.
                Err(e) => {
                    held = Some(e);
                    None
                }
            };
            // **Step 3 reads the expression's declared type, and it has to run before
            // the held failure.** `coerce_to_target_type` is a *type* verdict 18.6
            // reaches without evaluating anything, so a default that both fails
            // arithmetically and mistypes is the type error there:
            // `ADD COLUMN c INT DEFAULT (1/0)::text` and `… DEFAULT
            // (2147483647 + 1)::text` are both `42804` on 18.6 and were `22012` /
            // `22003` here (measured on both routes).
            crate::planner::check_assignment(
                expr,
                &column.name,
                column.type_oid,
                schema,
                crate::planner::TypeScope::NONE,
            )?;
            probe
        }
        // The one closed arm carrying a value at all. Every literal is probed, not
        // only the `Text` one an *unknown* literal produces: the assignment cast is
        // part of the cook whatever the constant's type, so `DEFAULT 1` into a `BOOL`
        // column is `42804` on 18.6 and was accepted-then-unwritable here. The
        // typmod-shaped neighbours (`12345.6` into `NUMERIC(4,2)`, `'abcdef'` into
        // `VARCHAR(3)`) stay accepted because their codes are outside
        // [`is_cook_time_error`], not because the probe skips them.
        //
        // **The assignment cast runs here off the `Value`'s own type, and that is safe
        // only because this arm is a bare constant by construction.** Postgres's
        // `unknown` is a syntactic property (`coercion::unknown_assignment_source`) and
        // a `Value` cannot carry it — but `parser::ddl`'s `column_default_from_expr`
        // builds this arm from an `Expr::Literal` alone, so its `Text` and `Null`
        // members *are* the unknowns and every other member is typed. Skipping them
        // keeps `int DEFAULT 'abc'` at 18.6's `22P02` (the target's input function,
        // run inside `coerce_to_target_type`) while `jsonb DEFAULT 5`,
        // `date DEFAULT 5` and `uuid DEFAULT 5` become its `42804` instead of the
        // `42846` the value coercion below was answering.
        ColumnDefault::Literal(value) => {
            if let Some(source) = value
                .type_oid()
                .filter(|_| !matches!(value, Value::Text(_)))
            {
                crate::planner::assignment_legality(source, column.type_oid, &column.name)?;
            }
            // An **unknown** literal into an enum column is converted by `enum_in` as
            // the default is cooked, on every route and whatever the rows: `DEFAULT 'q'`
            // over `mq AS ENUM ('a','b')` is `22P02` at `CREATE TABLE`, `SET DEFAULT`
            // and `ADD COLUMN` on 18.6, an empty table included (measured). The value
            // coercion below cannot say so — an enum value is a `Value::Text` — so the
            // membership test is this arm's, at the input function's position; left to
            // the write path it was `22P02` at the first insert taking the default and
            // **stored** on `ADD COLUMN`'s back-fill (`EDGES.md` row 105).
            if let (Value::Text(label), Some(enum_name)) = (value, column.enum_type.as_deref()) {
                crate::planner::enum_label_legality(enum_name, label, schema)?;
            }
            Some(value.clone())
        }
        _ => return Ok(()),
    };
    if let Some(e) = held
        && eager
    {
        return Err(e);
    }
    if let Some(probe) = probe {
        match coerce_column_value(probe, column) {
            Err(e) if eager || is_cook_time_error(&e) => return Err(e),
            // **What `ADD COLUMN` back-fills is a value the enum must hold**, whatever
            // spelled it. The existing rows take the coerced default without passing
            // `validate_column`, so a default the assignment gate cannot see as the
            // enum's — `'q' || ''`, `lower('Q')`, `'q'::text`, `5`, each `42804` on 18.6
            // (`EDGES.md` row 6) — was **stored** under an enum column (row 105's route).
            // Refused here as the enum's `22P02`, loud under the wrong code, on the eager
            // route alone, where the value is the rows': the deferred routes store no
            // row, and every write taking the default is refused by the write path.
            Ok(Value::Text(label)) if eager => {
                if let Some(enum_name) = column.enum_type.as_deref() {
                    crate::planner::enum_label_legality(enum_name, &label, schema)?;
                }
            }
            _ => {}
        }
    }
    // **The default must render**, and this is a pglet-only over-rejection so it runs
    // last *within the cook* — after the node walk, the probe's cook-time family, and
    // the coercion. Be exact about the scale, because the rule it is following is
    // stated at the statement scale and this does not reach it: the cook as a whole
    // sits at 18.6's own cook position, **ahead of the constraint naming block**, so over
    // `DEFAULT now() - interval '1 day'` this answers `0A000` where 18.6 reports the
    // duplicate CHECK name (`42710`) or the missing FK parent (`42P01`) behind it —
    // measured on both engines, along with the direction that is right (it loses to
    // the column type's `42704`). Moving it out would need a second per-default pass
    // beside `ensure_sequence_relation_for_new_table`, which is where the other
    // deliberate over-rejection on this path already runs; recorded in `EDGES.md` row
    // 47 rather than done here. A `ColumnDefault` is
    // persisted, and `pglet::dump` renders every persisted expression through one
    // `Result` — so a default `crate::deparse::render_expr` declines does not make
    // *that table* undumpable, it makes the **whole database** undumpable, which is
    // the accept-then-brick shape this engine refuses. The live instance is an
    // interval literal, which the parser carries as an `Expr::Bound`: 18.6 accepts
    // `DEFAULT now() - interval '1 day'` and pglet answers `0A000` at DDL rather than
    // recording a database that cannot be exported.
    //
    // **The invariant is *every accepted* expression default renders**, which is a
    // statement about the paths that reach this line and not about this line. It is
    // the last step of the cook, so any early return above it is a hole of exactly
    // the width of the arm that returns — and the deferred-eval arm was one:
    // `DEFAULT now() + interval '1 day' * (1/0)` failed its probe with `22012`, took
    // the deferred return, and persisted the very `Expr::Bound` the paragraph above
    // says is refused. The interval spelling that *evaluates* was `0A000` while its
    // divide-by-zero neighbour was accepted, after which `pglet_server dump` exited
    // 1 for the whole database — every other table's schema and rows lost from the
    // export — and `information_schema.columns.column_default` read SQL NULL for the
    // column. Both readers of a persisted default are guarded here and only here:
    // `pglet::dump`'s `render_default` and `information_schema`'s
    // `render_column_default` each say "no accepted DDL reaches the declining arm",
    // and this probe is what makes that true.
    //
    // **A column `DEFAULT` is one of four persisted-expression sites, and the other
    // three now run the same probe** (`validate_stored_expr_renders`) — this used
    // to be the only one, which left `CHECK (t > now() - interval '1 day')` accepted
    // and the whole database undumpable. **The statement-scale imperfection above is
    // now this site's alone**: the other three run their probe at the end of the
    // statement, where a gate standing in for no Postgres error belongs, and the ten
    // arrangements that used to diverge for them agree. Measured here, unchanged:
    // over `DEFAULT now() - interval '1 day'` this answers `0A000` where 18.6 reports
    // a duplicate CHECK name (`42710`), a missing FK parent (`42P01`) or a taken
    // constraint-index name (`42P07`) from the constraint naming block behind it — and
    // correctly loses to the column type's `42704`. Closing it is the same move:
    // a whole-statement pass over the cooked defaults, beside the sequence kind gate.
    // `EDGES.md` rows 47 and 49.
    if let ColumnDefault::Expr(expr) = default {
        crate::deparse::render_expr(expr)?;
        crate::depth::check_stored_expr(expr)?;
    }
    Ok(())
}

/// Whether `error` is one `cookDefault` raises rather than defers — the family
/// `cook_column_default` propagates from DDL time.
///
/// **The split is "resolution" vs "value", not "coercion" vs "the rest".** 18.6's
/// cook runs `transformExpr` (operator and type resolution) and then
/// `coerce_to_target_type` (the assignment cast), and a failure of *either* is a DDL
/// error; what it defers is a real cast function or an arithmetic operation failing
/// on the value it is handed. So the family is the type-resolution codes plus the
/// **input-function** codes — the input function being the one coercion Postgres
/// performs while cooking, because an `unknown` literal is folded into a typed
/// constant in place.
///
/// Measured on 18.6, all at `CREATE TABLE`: `'x'` into `INT` / `'notauuid'` into
/// `UUID` / `'notjson'` into `JSONB` are `22P02`, `'notats'` into `TIMESTAMP` is
/// `22007`, `'2026-13-01'` into `DATE` is `22008`, `'2000-01-01 00:00:00+16'` into
/// `TIMESTAMPTZ` is `22009`, `5 + 'x'` is `22P02`, and
/// `'x' || 'y'` into `INT` and `DEFAULT true` into `INT` are `42804`. The neighbours
/// it must **not** catch, all accepted there and raised per row: `22012` (`1/0`),
/// `22003` (`(2147483648)::int`, `12345.6` into `NUMERIC(4,2)`) and `22001`
/// (`'abcdef'` into `VARCHAR(3)`).
///
/// **The input-function codes are matched by code, not by the operand of the cast that
/// raised them.** 18.6's `coerce_type` calls a type's input function while cooking where
/// it coerces an `unknown` literal; a cast over an operand that already has a type — a
/// computed value (`('1' || 'x')::int`) or a typed constant (`'x'::text::int`,
/// `text 'x'::int`) — becomes a `CoerceViaIO` evaluated with the default. So those defaults
/// are refused at `CREATE TABLE` here where 18.6 creates the table and refuses each write
/// that uses the default (`EDGES.md` row 47). The bare-literal arm reads this predicate too,
/// so dropping a code would accept the bare literal 18.6 refuses. The divergence runs the
/// other way as well, and not through this predicate: the probe evaluates only the branches
/// it takes, so a cast over an `unknown` literal in an untaken branch
/// (`CASE WHEN false THEN 'x'::int ELSE 1 END`) raises nothing here to match, where 18.6
/// refuses it while cooking (row 47).
///
/// **The resolution codes are what stops an accept-then-brick**, and leaving them
/// out was measured doing exactly that: `DEFAULT 5 + 'x'` and `DEFAULT 1 + 1` into a
/// `BOOL` column were both *accepted*, and then every omitted-column `INSERT` failed
/// (`42804` / `42846`) — a column defined and unwritable, which is the shape 18.6
/// refuses at DDL. pglet's codes are its own on those two (`42804` where 18.6 says
/// `22P02`, `42846` where it says `42804`); both are loud and both land on the same
/// statement.
fn is_cook_time_error(error: &Error) -> bool {
    matches!(
        error.sqlstate(),
        // Type input functions, run at parse analysis on an `unknown` literal.
        "22P02" | "22007" | "22008" | "22009"
        // Operator / type resolution and the assignment cast.
        | "42883" | "42804" | "42846"
        // pglet's own unsupported-construct arm, which stands in for a construct it
        // cannot evaluate at all — an accepted default that fails every write.
        | "0A000"
    )
}

/// Whether `expr` is the synthesized bare star an argument-less aggregate carries —
/// `count(*)`'s `*`, which is not a column reference. A qualified `t.*` is one, and
/// so is any named column.
fn is_bare_star(expr: &Expr) -> bool {
    matches!(expr, Expr::Column { table: None, name } if name == "*")
}

/// Walk a column `DEFAULT` expression **bottom-up, left-to-right**, applying the
/// per-node verdicts 18.6 raises in parse analysis: the constructs it forbids in a
/// `DEFAULT`, then the shared operand-type rulebook.
///
/// **One walk with one verdict per node**, because `transformExprRecurse` resolves
/// children before parents and the left operand before the right, so the
/// *positionally first* offending node wins whatever kind it is. Ranking the kinds
/// instead — the shape `parser::ddl`'s `reject_persisted_expr_constructs` still has
/// for the three CHECK/index contexts — gets half of every mirrored pair wrong.
/// Measured on 18.6 over `d(a INT, b INT DEFAULT …)`:
///
/// | default expression | 18.6 |
/// |---|---|
/// | `(SELECT 1) + a` | `0A000` subquery |
/// | `a + (SELECT 1)` | `0A000` **column reference** |
/// | `count(*) + a` | `42803` aggregate |
/// | `a + count(*)` | `0A000` column reference |
/// | `count(*) + (SELECT 1)` | `42803` |
/// | `(SELECT 1) + count(*)` | `0A000` subquery |
/// | `row_number() OVER () + a` | `42P20` window |
/// | `a + row_number() OVER ()` | `0A000` column reference |
/// | `unnest(ARRAY[1,2]) + a` | `0A000` set-returning |
/// | `a + unnest(ARRAY[1,2])` | `0A000` column reference |
/// | `$1 + a` | `42P02` |
/// | `a + $1` | `0A000` column reference |
///
/// Three node kinds do **not** simply recurse-then-decide, and each is measured:
///
/// - **A subquery-bearing node is opaque, verdict at the node.**
///   `transformSubLink` raises before it transforms the sub-select *or* the
///   `testexpr`, so `a IN (SELECT 1)` — whose left operand is a column reference —
///   is `0A000 cannot use subquery`, not the column-reference message.
/// - **A window function's `OVER` clause is not walked**, because
///   `transformWindowFuncCall` raises before the `WindowDef` is resolved:
///   `row_number() OVER (PARTITION BY a)` and `… OVER (ORDER BY a)` are `42P20`,
///   not `0A000`. [`Expr::for_each_child`] *does* descend into `partition_by` /
///   `order_by`, so delegating to it here would answer the column-reference error.
///   Postgres does walk a window function's **arguments** (`sum(a) OVER ()` is
///   `0A000` column reference), but pglet's [`Expr::Window`] carries no argument
///   expression at all — `WindowFunc` is the three niladic ranking functions — so
///   that half of the rule has no representable instance here and no fixture can
///   pin it; it is stated for the walk that will exist once one does.
/// - **`count(*)`'s star is not a column reference, and the exemption belongs to the
///   aggregate rather than to the star.** The star is spelled
///   `Expr::Column { table: None, name: "*" }` in this IR, so an unconditional
///   column-reference verdict would fire on it and answer `0A000` where 18.6 says
///   `42803`. So an `Expr::Aggregate` skips a **direct** bare-star argument and walks
///   the rest, then raises: `count(*)` is `42803` while `count(t.*)` and `count(a)`
///   are `0A000 cannot use column reference`, all three measured on 18.6.
///   Exempting the star *wherever it appears* — which is what a `Column`-arm guard
///   does — is wrong in a way that only became reachable with the `Expr` catch-all:
///   `DEFAULT "*"` and `DEFAULT ("*" + 1)` would have passed the walk and **persisted
///   a column reference into the default**, which every write then fails to resolve.
///   18.6 answers `0A000 cannot use column reference` to both, and so does this.
///   What the aggregate-scoped exemption still gets wrong is the *forged* star: a
///   user's quoted `"*"` column is byte-identical to the synthesized one, so
///   `count("*")` and `("*" + count(*))` are `42803` here against 18.6's `0A000`.
///   Loud both ways, the same quoted-forgery class as the `"a.b"` / `"PUBLIC"`
///   instances in `docs/scope.md` / `docs/sequences.md`, and closing it needs a dedicated star marker in the
///   IR rather than a name.
///
/// The column-reference verdict is **unconditional** — existence is never consulted,
/// because `transformColumnRef` refuses before resolving the name. `DEFAULT
/// totally_bogus_name` and `DEFAULT <a real sibling>` are the same `0A000` on both
/// engines, and there is no `42703` in this context at all. That is the opposite of
/// the CHECK / index contexts, where a column reference is legal and only an unknown
/// one raises — which is why this is separate code from
/// [`check_stored_expr_node_binding`], and why the operand-type check below is
/// handed an **empty** column set: a `DEFAULT` expression that survives the walk
/// references no relation at all.
fn check_default_expr_nodes(expr: &Expr) -> Result<()> {
    const CTX: crate::parser::PersistedExprContext = crate::parser::PersistedExprContext::Default;
    crate::depth::check_stack()?;
    // Verdict at the node, children never reached (see the doc's first two bullets).
    match expr {
        Expr::Subquery { .. } | Expr::Exists { .. } | Expr::InSubquery { .. } => {
            return Err(Error::SubqueryNotAllowed {
                context: CTX.singular(),
            });
        }
        Expr::Window { .. } => {
            return Err(Error::WindowingError {
                context: CTX.plural(),
            });
        }
        _ => {}
    }
    // Children first, in source order, then this node — except an aggregate's own
    // bare-star argument, which is not a column reference (see the doc's third
    // bullet); every other argument is walked.
    if let Expr::Aggregate { arg, .. } = expr {
        if !is_bare_star(arg) {
            check_default_expr_nodes(arg)?;
        }
    } else {
        expr.try_for_each_child(check_default_expr_nodes)?;
    }
    match expr {
        Expr::Aggregate { .. } => {
            return Err(Error::AggregateNotAllowed {
                context: CTX.plural(),
            });
        }
        Expr::Unnest(_) => {
            return Err(Error::SetReturningNotAllowed {
                context: CTX.plural(),
            });
        }
        Expr::Column { .. } => {
            return Err(Error::ColumnReferenceNotAllowed {
                context: CTX.singular(),
            });
        }
        Expr::Parameter { index } => return Err(Error::UndefinedParameter { index: *index }),
        // 18.6: `DEFAULT is not allowed in this context` (`42601`), the same sentence
        // the stored-expression binding check raises for `CHECK (a = DEFAULT)`.
        Expr::Default => {
            return Err(Error::syntax(super::DEFAULT_NOT_ALLOWED.to_string()));
        }
        _ => {}
    }
    // No enum member list, and no identity for one to be tested against: this walk
    // refuses an `Expr::Column` outright (above), so the identity walk's column leaf
    // cannot answer here and its subquery leaf reads this same empty schema. A cast to an
    // enum carries its own member list (`StringFunc::EnumIn`), which the operand-type walk
    // below reads for a literal; a bare label into an enum column is the cook's `Literal`
    // arm, and a `text`-typed default into one the assignment gate's (`EDGES.md` row 6).
    let empty = probe_schema(&[], &BTreeMap::new());
    check_expr_node_operand_types(expr, &empty, Some(PROBE_TABLE))?;
    let probe_scope = PlannerScope::catalog(&empty, Some(PROBE_TABLE));
    branch_unification_legality(expr, &|e| probe_scope.resolved_type(e), &|_| false, &empty)
}

/// Validate a **stored expression** at DDL time — an index partial predicate, an
/// index key expression, or a CHECK constraint — so a failure lands where Postgres
/// puts it (at `CREATE INDEX` / `CREATE TABLE` / `ALTER TABLE`) rather than on some
/// later write.
///
/// Without this, pglet accepted `CREATE INDEX … WHERE int_col LIKE '1%'` and then
/// **every subsequent INSERT failed**, since index maintenance evaluates the stored
/// predicate and the operand gate rejects it there. An accepted-but-unwritable table
/// is strictly worse than a rejected DDL — and it is worse still than it sounds,
/// because a table carrying an unrenderable stored expression also makes
/// `dump_sql` fail for the **whole database**. That second cost is
/// [`validate_stored_expr_renders`]'s to prevent, not this function's; it is a
/// separate function because it runs at a different statement position, and the
/// sentence above is what the pair together makes true.
///
/// Three complementary checks run, because none subsumes the others:
///
/// 1. **Binding** ([`check_stored_expr_node_binding`]) — every leaf must be
///    resolvable in this context at all: no `DEFAULT` (`42601`), no `$N` parameter
///    (`42P02`), no column the table does not have (`42703`).
/// 2. **Static operand typing** ([`check_expr_node_operand_types`]) — the shared
///    legality rules over `Expr::infer_type`. This is the load-bearing one. A stored
///    expression carries no bound parameters, so every leaf resolves and
///    `data->'k'` is `jsonb` regardless of what any row holds.
///
/// The first two are *per-node* verdicts run from one shared walk
/// ([`check_stored_expr_nodes`]), not two passes — see there for why the ordering is
/// load-bearing.
///
/// 3. **Probe evaluation** — one synthetic row of representative non-NULL values,
///    evaluated through the same `eval_expr` the maintenance path uses. Only an
///    *operator-resolution* outcome is propagated — `UndefinedFunction` (`42883`)
///    and `NotImplemented` (`0A000`). Every other error is a **value**-dependent
///    outcome this synthetic row has no business deciding (a bad cast, an overflow,
///    a division by zero), so it is swallowed and left to real writes.
///
/// **The guarantee, stated honestly.** The static check catches what values cannot
/// reveal: an intermediate that yields SQL NULL makes every downstream gate
/// NULL-propagate, so the probe never fires for `WHERE data->'k' LIKE 'x%'` — `->`
/// on an object lacking key `k` is NULL for *every* representative `data`, and no
/// choice of representative value fixes that, since the probe cannot know which key
/// the predicate names. The probe conversely catches what the static rules don't
/// model: any loud operand failure raised by an eval path outside the five legality
/// families. Together they are a strong filter, **not** a proof — an expression both
/// accept can still fail at write time on a value-dependent error, which is correct
/// (Postgres does the same) but means "accepted here" is not "never errors later".
///
/// **The known accepted-but-unwritable hole is this filter's own.** The match below
/// propagates only `UndefinedFunction`/`NotImplemented`, so a **constant cast** — which
/// the probe reaches and fails on — is discarded *by kind* and the DDL is
/// accepted: `CHECK (upper('a'::int) = 'A')` is `22P02` on 18.6 and taken here, at every
/// route reaching this function — measured on an earlier build and here at all five CHECK
/// spellings ([`validate_check_exprs`]: column-level and table-level on `CREATE TABLE`,
/// `ALTER TABLE … ADD CONSTRAINT … CHECK`, the bare `ALTER TABLE … ADD CHECK`, and
/// `ADD COLUMN`'s inline one) and both index positions ([`validate_index_exprs`]). What makes it *silent* is
/// the **row count** and not the route: over one row each route that can meet one answers
/// `22P02` here too, from the back-fill and the existing-row validation, so the empty
/// table — the ordinary migration shape — is the one that gets through, and
/// `CREATE TABLE`'s own inline CHECK gets through always. That the probe *runs and
/// reaches eval* at these routes is measured and not assumed: over the same empty table
/// `ADD CONSTRAINT n CHECK (to_char(1.5,'9.9') = 'x')`, whose eval raises
/// `NotImplemented`, is refused `0A000` here where the constant-cast CHECK beside it is
/// taken — only the kind separates them. It leaves the unwritable table
/// above and, beside it, the silent counterpart of the undumpable database
/// [`validate_stored_expr_renders`] prevents: `pglet_server dump` exits `0` and emits
/// `CREATE INDEX dmp_ix ON dmp ((upper(('a')::integer)))` and its `ADD CONSTRAINT` twins,
/// which stock 18.6 refuses `22P02` while loading every `CREATE TABLE` and `COPY` — so the
/// restore keeps the tables and rows and quietly loses the index and the constraints,
/// where the render probe's failure is at least loud. pglet's own restore of that script
/// reports nothing at all, those tables being empty. A constant cast is not the
/// value-dependent outcome this filter is written for; the repair belongs here and is
/// queued, recorded in `EDGES.md` with the routes and the
/// condition.
///
/// **The known false-*rejection*.** A stored expression is checked as parsed, without
/// the planner's literal-coercion pass, so a bare string literal types as `text`
/// rather than staying Postgres's `unknown`. Postgres would resolve it from the
/// operator signature; pglet gates it as text. So `CHECK (n = '5')`,
/// `CHECK (d @> '{"k":1}')`, and their siblings across every gated family are
/// rejected here and accepted there — the fix is the explicit cast (`'5'::int`,
/// `'{"k":1}'::jsonb`). Deliberately not "fixed" by treating a literal as unknown:
/// the legality rules accept any pair with an unknown side, which would also stop
/// rejecting `CHECK (data->'k' LIKE 'x%')` — the NULL-intermediate shape the probe
/// structurally cannot see, and the reason the static check exists. Closing it
/// properly means resolving the stored expression against the table's column scope
/// before it is recorded, not loosening the rules. Until then the error lands at DDL
/// time, in the safe direction.
///
/// **A fourth check is not here, deliberately**: the render probe
/// ([`validate_stored_expr_renders`]) stands in for **no** Postgres error, so it
/// cannot inherit these three's statement position. See its own doc.
fn validate_stored_expr(
    expr: &Expr,
    columns: &[Column],
    enums: &BTreeMap<String, EnumType>,
) -> Result<()> {
    check_stored_expr_nodes(expr, &probe_schema(columns, enums), columns)?;
    let names: Vec<&str> = columns.iter().map(|c| c.name.as_str()).collect();
    let types: Vec<TypeOid> = columns.iter().map(|c| c.type_oid).collect();
    let row: Vec<Value> = columns
        .iter()
        .map(|c| representative_value(c.type_oid))
        .collect();
    // No enum orders: this probe only asks whether the expression *resolves* (it keeps
    // only `UndefinedFunction` / `NotImplemented`), and a comparison's verdict is not
    // read, so the member order cannot change its answer.
    match super::expr::eval_expr(expr, &names, &types, &[], &row) {
        Err(e @ (Error::UndefinedFunction(_) | Error::NotImplemented(_))) => Err(e),
        _ => Ok(()),
    }
}

/// The **render probe** on a stored expression — a CHECK predicate, an index partial
/// predicate, or an index key part — the twin of the one
/// [`cook_column_default`] runs on a column `DEFAULT`, and the fourth check
/// [`validate_stored_expr`] deliberately does not carry.
///
/// `pglet::dump` renders **every** persisted expression through one `Result`, so an
/// expression the deparser declines does not make that one table undumpable — it
/// makes the **whole database** undumpable: `pglet_server dump` exits 1 and every
/// other table's schema and rows are lost from the export. Reproduced on the tree
/// before this existed, at all five CHECK spellings (column-level and table-level on
/// `CREATE TABLE`, `ALTER TABLE … ADD CHECK` and `… ADD CONSTRAINT … CHECK`, and
/// `ADD COLUMN`'s inline one — three call sites) and both index positions, from one
/// spelling each: `CREATE TABLE ck1 (t TIMESTAMPTZ, CHECK (t > now() - interval '1
/// day'))` — ordinary DDL 18.6 accepts — and `CREATE INDEX … ((iv + interval '1
/// day'))` / `… WHERE iv > interval '1 day'` over an `INTERVAL` column, likewise
/// accepted there. The declining variant is `Expr::Bound`, which the parser produces
/// for an `INTERVAL '…'` literal (`parser::expr`'s `parse_interval_literal`) and
/// which is the only live SQL-text producer of it — `parser::ddl`'s
/// `column_default_to_expr` holds a second constructor, dead by its own classifier's
/// range and confined to the `DEFAULT` site [`cook_column_default`] already probes.
///
/// **It is a separate function from [`validate_stored_expr`] because it is a
/// separate *whose-error-is-it*, not a separate *what-it-inspects*.** The three
/// checks there each stand in for an error 18.6 raises in parse analysis, ahead of
/// `index_create`'s name conflict, so they run where they do. This one stands in for
/// no Postgres error at all — 18.6 accepts every expression it declines — so by the
/// engine's rule it must run after every gate that stands in for one, *including the
/// ones that decide a Postgres success*. The split mirrors
/// [`validate_index_method`] (Postgres's half, before the name) versus
/// [`validate_btree_key_type`] (pglet's own, after the name and the `IF NOT EXISTS`
/// skip); folding this into `validate_stored_expr` was measured turning
/// `CREATE INDEX IF NOT EXISTS <a taken name> ON t ((iv + interval '1 day'))` — the
/// idempotent-migration idiom, and a **success** on 18.6 — into a `0A000`.
///
/// **On the three CHECK routes the same rule puts it in a whole-statement pass of
/// its own** ([`validate_check_exprs_render`]), after every gate that stands in for a
/// real 18.6 error and immediately before the first catalog write. Riding inside
/// [`validate_check_exprs`] — one constraint's cook, as the three checks above do —
/// was measured wrong in **ten** arrangements across the three routes: an interval
/// CHECK pre-empted a taken constraint-index name (`42P07`), a missing FK parent
/// (`42P01`), a duplicate CHECK name (`42710`), an `ADD CONSTRAINT`'s existing-row
/// violation and an `ADD COLUMN`'s backfill violation (`23514`), each answering
/// `0A000` here against 18.6's own code. Only the duplicate-name kind was
/// order-sensitive — it lives in the same per-constraint cook-then-name loop, so
/// writing the duplicate *first* already agreed — while the constraint-index and FK
/// kinds diverged in **both** source orders, the whole CHECK loop standing ahead of
/// them. The pass is per statement rather than per constraint because nothing 18.6
/// raises for these expressions can be positioned against them: it accepts every one,
/// so the probe's only correct position is behind everything.
fn validate_stored_expr_renders(expr: &Expr) -> Result<()> {
    crate::deparse::render_expr(expr)?;
    // The same position, for the same reason: the catalog encoding's walk is pglet's
    // own bound, not an 18.6 error (`crate::depth`).
    crate::depth::check_stored_expr(expr)
}

/// Walk a stored expression **bottom-up, left-to-right**, applying both static
/// per-node verdicts at each node: the DDL-only binding check
/// ([`check_stored_expr_node_binding`]) and the shared operand-type rulebook
/// ([`check_expr_node_operand_types`]). The first failure in that order is the one
/// reported.
///
/// **One walk, not two passes — the order is the whole point.** Postgres resolves a
/// stored expression during parse analysis: children before parents, left operand
/// before right, so the *positionally first* failure wins whichever kind it is.
/// Verified against 18.6:
///
/// | stored expression                  | Postgres |
/// |------------------------------------|----------|
/// | `CHECK (a LIKE 'x' AND bogus > 0)` | `42883`  |
/// | `CHECK (bogus > 0 AND a LIKE 'x')` | `42703`  |
/// | `CHECK ((a -> 'k') LIKE 'x')`      | `42883`, naming the *inner* `integer -> …` |
///
/// Run as two whole-tree passes — bindings, then operand types — the binding error
/// would win in all three regardless of position, and swapping the passes just moves
/// the divergence to the other side. Only a single interleaved traversal produces
/// Postgres's answer in all three, which is why the recursion lives here rather than
/// inside either check.
///
/// Child order follows `Expr::for_each_child`, which is **source order for every
/// variant** — including `Expr::Extract`, whose declared field order (`source`, then
/// `field`) is the opposite of both its spellings and whose walker therefore yields
/// `field` first on purpose. That inversion mattered here: whichever slot the walk
/// reaches first supplies the reported error, and `date_part(field, source)` takes an
/// arbitrary expression in *either* slot, so a wrong order flips the verdict in both
/// directions (`tests/fixtures/ddl/check_date_part_*`).
fn check_stored_expr_nodes(expr: &Expr, schema: &Schema, columns: &[Column]) -> Result<()> {
    crate::depth::check_stack()?;
    // A child in a **boolean context** is gated the moment its own subtree is done,
    // before the next child is walked — 18.6's `transformBoolExpr` /
    // `transformCaseExpr` coerce one argument before transforming the next, so
    // `CHECK (a AND b LIKE 'x')` is `42804 argument of AND` and `CHECK (b LIKE 'x'
    // AND a)` is `42883`. Both measured; no pair of whole-subtree passes yields both.
    expr.try_for_each_child_ctx(|child, ctx| {
        check_stored_expr_nodes(child, schema, columns)?;
        ctx.map_or(Ok(()), |ctx| {
            check_boolean_operand(
                child,
                ctx,
                &PlannerScope::catalog(schema, Some(PROBE_TABLE)),
            )
        })
    })?;
    check_stored_expr_node_binding(expr, columns)?;
    check_expr_node_operand_types(expr, schema, Some(PROBE_TABLE))?;
    // The construct-level half of the same rulebook: a `COALESCE`/`CASE`/`ARRAY` whose
    // branches have no common type. A stored expression is exactly where this must be
    // caught up front — `CHECK (COALESCE(int_col, text_col) IS NOT NULL)` is `42804` in
    // Postgres, and recording it here instead left a constraint that could never mean
    // anything. Nothing defers in a stored expression: there is no enclosing query
    // level to resolve a branch later, and an unbindable column was already rejected by
    // the binding check above.
    let probe_scope = PlannerScope::catalog(schema, Some(PROBE_TABLE));
    branch_unification_legality(expr, &|e| probe_scope.resolved_type(e), &|_| false, schema)
}

/// Reject **this node** if it is a leaf that can never bind in a stored expression:
/// `DEFAULT`, a `$N` parameter, or a column the table does not have. Every other node
/// is `Ok`; the caller ([`check_stored_expr_nodes`]) drives the traversal.
///
/// All three are **accept-then-brick** shapes — pglet used to record them and then
/// fail every write to the table, because the stored expression is re-evaluated on
/// each INSERT/UPDATE with no default context, no parameter list, and no such
/// column. The table became permanently unwritable *and* the whole database
/// undumpable (`pglet::dump` renders every stored expression through one `Result`).
/// Postgres rejects all three at DDL time, at each of the three sites, with the
/// SQLSTATEs matched here — verified against 18.6:
///
/// | shape                | Postgres message                    | SQLSTATE |
/// |----------------------|-------------------------------------|----------|
/// | `CHECK (a = DEFAULT)`| `DEFAULT is not allowed in this context` | `42601` |
/// | `CHECK (a > $1)`     | `there is no parameter $1`          | `42P02`  |
/// | `CHECK (bogus > 0)`  | `column "bogus" does not exist`     | `42703`  |
///
/// **Static, not probe-based, on purpose.** The probe half of [`validate_stored_expr`]
/// reaches these errors only when evaluation actually visits the leaf — a
/// short-circuiting `AND`/`OR` or a NULL-propagating intermediate can walk past it —
/// and it deliberately swallows every error outside the operator-resolution pair. A
/// structural walk has neither hole.
///
/// **Scope.** Only the stored-expression path calls this. `Expr::Default` stays legal
/// in `INSERT … VALUES (DEFAULT)` / `UPDATE … SET c = DEFAULT` and `Expr::Parameter`
/// everywhere a bound statement runs; neither goes through here.
///
/// The column test matches on the **name** only, ignoring any table qualifier, so
/// `CHECK (t.a > 0)` keeps working — a qualifier naming some *other* relation is a
/// separate divergence (Postgres: `42P01`, missing FROM-clause entry) that this does
/// not address.
fn check_stored_expr_node_binding(expr: &Expr, columns: &[Column]) -> Result<()> {
    match expr {
        Expr::Default => Err(Error::syntax(super::DEFAULT_NOT_ALLOWED.to_string())),
        Expr::Parameter { index } => Err(Error::UndefinedParameter { index: *index }),
        Expr::Column { name, .. } if !columns.iter().any(|c| c.name == *name) => {
            Err(crate::CatalogError::ColumnNotFoundInExpr {
                column: name.clone(),
            }
            .into())
        }
        _ => Ok(()),
    }
}

/// The table name the DDL-time inference scope is keyed under. Arbitrary — the
/// synthetic schema holds exactly one table, and a stored expression's column
/// references are either bare or qualified by the real table name (which the
/// resolver falls back past, see `Expr::infer_type`).
const PROBE_TABLE: &str = "";

/// A one-table `Schema` over `columns`, so `Expr::infer_type` — the *same*
/// inference the planner runs — resolves this expression's column references.
///
/// Built rather than passing the live schema because the checked table may not be
/// in it yet (`CREATE TABLE`'s inline CHECKs) or may not yet carry the new column
/// (`ALTER TABLE … ADD COLUMN`'s inline CHECKs).
///
/// `enums` is the **live** catalog's, carried in rather than defaulted: an enum's
/// *identity* rides on each [`Column`] and so arrives with `columns`, but its **member
/// list** is a database-level fact, and the rule that reads it
/// (`planner::enum_literal_legality`) is what stops a persisted `CHECK (m > 'zzz')`
/// from dumping to SQL stock 18.6 refuses to load. A synthetic schema with an empty
/// `enums` map declines that test silently, which is what it did.
fn probe_schema(columns: &[Column], enums: &BTreeMap<String, EnumType>) -> Schema {
    let mut schema = Schema {
        enums: enums.clone(),
        ..Schema::default()
    };
    schema.tables.insert(
        PROBE_TABLE.to_string(),
        Table {
            name: PROBE_TABLE.to_string(),
            schema: "public".to_string(),
            columns: columns.to_vec(),
            oid: 0,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
        },
    );
    schema
}

/// A representative **non-NULL** value of `oid`, for the DDL-time probe in
/// [`validate_stored_expr`].
///
/// Non-NULL is the point: the operand gates short-circuit on NULL (SQL NULL
/// propagation), so an all-NULL probe row would type-check nothing. The particular
/// value is irrelevant — the gates dispatch on the `Value` *variant* — so each is
/// simply the cheapest inhabitant of its type.
fn representative_value(oid: TypeOid) -> Value {
    use crate::types::{ArrayValue, JsonbValue, PgInterval, PgNumeric};
    match oid {
        TypeOid::Bool => Value::Bool(false),
        TypeOid::Int2 => Value::Int2(0),
        TypeOid::Int4 => Value::Int4(0),
        TypeOid::Int8 => Value::Int8(0),
        TypeOid::Float4 => Value::Float4(0.0),
        TypeOid::Float8 => Value::Float8(0.0),
        TypeOid::Text | TypeOid::Varchar => Value::Text(String::new()),
        TypeOid::Bytea => Value::Bytea(Vec::new()),
        TypeOid::Timestamp | TypeOid::Timestamptz => {
            Value::timestamp(time::OffsetDateTime::UNIX_EPOCH)
        }
        TypeOid::Date => Value::date(time::OffsetDateTime::UNIX_EPOCH.date()),
        TypeOid::Time => Value::Time(0),
        TypeOid::Interval => Value::Interval(PgInterval {
            months: 0,
            days: 0,
            micros: 0,
        }),
        TypeOid::Numeric => Value::Numeric(PgNumeric::zero()),
        TypeOid::Uuid => Value::Uuid([0; 16]),
        TypeOid::Jsonb => Value::Jsonb(JsonbValue::Null),
        // An array type: an empty array carrying its element type (the element type
        // is what an array-aware gate dispatches on, and `array_element` is total
        // over the array OIDs).
        other => Value::Array(ArrayValue {
            element_type: other.array_element().unwrap_or(TypeOid::Text),
            elements: Vec::new(),
        }),
    }
}

/// The **clause-root** boolean gate for a stored predicate — a `CHECK` or a partial
/// index's `WHERE` — where 18.6's `cookConstraint` / `transformIndexStmt` raise it:
/// after the whole expression is transformed, and before anything `DefineIndex` or
/// `heap.c` does with the result.
///
/// It runs **after** [`validate_stored_expr`]'s node walk and probe, which is 18.6's
/// order (`coerce_to_boolean` is the last thing `cookConstraint` does to the
/// expression) and is also what keeps the probe's recorded `0A000` shapes —
/// `CHECK (unnest(intcol))` among them — answering as they do rather than being
/// pre-empted by a `42804` on a result type 18.6 never reaches.
///
/// Without it a stored predicate accepted here is one stock Postgres refuses to
/// load, which breaks the dump invariant's catalog part a persisted expression exists
/// under: any catalog pglet's own accepted statements build must dump to something that
/// loads into stock Postgres and back into pglet.
///
/// **The probe schema is built with the catalog's enum member lists**, like
/// [`validate_stored_expr`]'s. Its one consumer today asks no membership question, so an
/// empty map was benign — and benign is not a property a caller list keeps: the
/// byte-identical omission two hundred lines up let a non-member label into a persisted
/// `CHECK`, which then dumped to SQL stock 18.6 refuses while `psql` exited `0`. Passing
/// the real map here costs one argument and removes the seam rather than re-proving it.
///
/// # Errors
///
/// [`Error::Type`] (`42804`) for a non-boolean predicate; `22P02` for an unresolved
/// literal that is not a boolean spelling.
fn validate_stored_predicate(
    expr: &Expr,
    ctx: BoolContext,
    columns: &[Column],
    enums: &BTreeMap<String, EnumType>,
) -> Result<()> {
    let schema = probe_schema(columns, enums);
    check_boolean_operand(
        expr,
        ctx,
        &PlannerScope::catalog(&schema, Some(PROBE_TABLE)),
    )
}

/// Validate every stored expression a **CHECK constraint** list carries, against
/// `columns` (see [`validate_stored_expr`]).
///
/// Takes the parse-side [`CheckConstraintDef`] because that is where this runs —
/// before the name is minted, on the way into the catalog.
///
/// The render probe is **not** here: it stands in for no 18.6 error, so it runs at
/// the end of the statement instead ([`validate_check_exprs_render`]).
fn validate_check_exprs(
    checks: &[CheckConstraintDef],
    columns: &[Column],
    enums: &BTreeMap<String, EnumType>,
) -> Result<()> {
    for check in checks {
        validate_stored_expr(&check.expr, columns, enums)?;
        validate_stored_predicate(&check.expr, BoolContext::Check, columns, enums)?;
    }
    Ok(())
}

/// [`validate_check_exprs`]'s render half — the whole-statement pass, run once every
/// gate standing in for a real 18.6 error has passed and immediately before the
/// statement's first catalog write. See [`validate_stored_expr_renders`] for why the
/// position is the whole of it, and [`validate_index_exprs_render`] for the index
/// routes' twin.
///
/// Takes the **catalog** [`CheckConstraint`] rather than the parse-side def, because
/// by this point in each of the three routes the names are minted and that is the
/// list in hand; the expression is the same object either way.
fn validate_check_exprs_render(checks: &[CheckConstraint]) -> Result<()> {
    for check in checks {
        validate_stored_expr_renders(&check.expr)?;
    }
    Ok(())
}

/// Every stored expression an index carries, in the order the checks visit them: the
/// partial `predicate`, then each key part of an expression key.
///
/// **One enumeration, two consumers** ([`validate_index_exprs`] and
/// [`validate_index_exprs_render`]), because those two run at different statement
/// positions and a key kind added to one loop and not the other would leave the
/// second silently narrower. A plain [`IndexKey::Columns`] key carries no expression.
///
/// Measured as two mutations, and the blast radius is the argument for the shared
/// shape: dropping the key `parts` reddens five fixtures — the four `ddl/index_key_*`
/// plus `ddl/index_nested_array_expr_loud` — and the two dump guards
/// (`a_database_with_any_accepted_stored_expression_dumps`,
/// `stored_expression_sites_reject_leaves_that_cannot_bind`); dropping the
/// `predicate` reddens the six executor-side `ddl/index_predicate_*` (the subquery
/// and window ones are parser-side refusals and never reach this walk) and the same
/// two guards — in each case the *type/binding* checks and the render probe alike,
/// and nothing of the other position. A per-consumer enumeration could have lost either
/// half on its own with the other still green.
fn index_stored_exprs<'a>(
    key: &'a IndexKey,
    predicate: Option<&'a Expr>,
) -> impl Iterator<Item = &'a Expr> {
    let parts: &'a [Expr] = match key {
        IndexKey::Exprs(parts) => parts,
        IndexKey::Columns(_) => &[],
    };
    predicate.into_iter().chain(parts)
}

/// Validate an index's stored expressions — its partial `predicate` and, for an
/// expression index, each key part — against `columns` (see
/// [`validate_stored_expr`]), plus the predicate's own clause-root boolean gate.
fn validate_index_exprs(
    key: &IndexKey,
    predicate: Option<&Expr>,
    columns: &[Column],
    enums: &BTreeMap<String, EnumType>,
) -> Result<()> {
    for expr in index_stored_exprs(key, predicate) {
        validate_stored_expr(expr, columns, enums)?;
    }
    // 18.6 spells the partial predicate's construct `"WHERE"`
    // (`parse_utilcmd.c`'s `transformIndexStmt`), and it is the **predicate**'s, not
    // the key's — a key part has no boolean context at all.
    if let Some(predicate) = predicate {
        validate_stored_predicate(predicate, BoolContext::Where, columns, enums)?;
    }
    Ok(())
}

/// Refuse an index predicate or key expression that holds a **conversion to an enum** or
/// a call of **`random()`** — `42P17 functions in index {predicate,expression} must be
/// marked IMMUTABLE`, 18.6's answer, `enum_in` being `STABLE` there and `random`
/// `VOLATILE`.
///
/// pglet models no function volatility and accepts an index over `now()` or
/// `current_user` where 18.6 refuses it (the permissiveness `deparse::render_expr`'s doc
/// records, `EDGES.md` row 285). These two members of that family are refused here: the
/// enum conversion because the node persists naming the type — accepted, the index gates
/// writes on the frozen member list and dumps to a `CREATE INDEX` stock Postgres refuses
/// on reload — and `random()` because a key drawn afresh at each evaluation is one no
/// lookup can find again.
///
/// A cast whose operand is **already** that enum is a relabel 18.6 plans as no function
/// at all (`CREATE INDEX … ((m::mood))` over a `mood` column is accepted, measured), so it
/// passes. A member literal (`'ok'::mood`) is not a conversion node here in the first
/// place, and a non-member one was refused `22P02` in parse analysis, ahead of this.
fn refuse_mutable_index_functions(
    key: &IndexKey,
    predicate: Option<&Expr>,
    schema: &Schema,
    table: &str,
) -> Result<()> {
    let scope = PlannerScope::catalog(schema, Some(table));
    let mutable = |e: &Expr| holds_enum_conversion(e, &scope) || holds_random(e);
    if predicate.is_some_and(mutable) {
        return Err(Error::IndexFunctionNotImmutable {
            position: "predicate",
        });
    }
    if let IndexKey::Exprs(exprs) = key
        && exprs.iter().any(mutable)
    {
        return Err(Error::IndexFunctionNotImmutable {
            position: "expression",
        });
    }
    Ok(())
}

/// Whether `expr` holds a call of `random()` anywhere in its scalar tree.
fn holds_random(expr: &Expr) -> bool {
    if matches!(expr, Expr::Builtin(crate::planner::BuiltinFn::Random)) {
        return true;
    }
    let mut found = false;
    expr.for_each_child(|child| found = found || holds_random(child));
    found
}

/// Whether `expr` holds a conversion to an enum whose operand is not already that enum.
fn holds_enum_conversion(expr: &Expr, scope: &PlannerScope<'_>) -> bool {
    if let Some((operand, name, _)) = crate::planner::enum_in_parts(expr) {
        let relabel = scope
            .resolved_type(operand)
            .is_some_and(|t| t.enum_type.name() == Some(name));
        if !relabel {
            return true;
        }
    }
    let mut found = false;
    expr.for_each_child(|child| {
        if !found && holds_enum_conversion(child, scope) {
            found = true;
        }
    });
    found
}

/// [`validate_index_exprs`]'s render half, split out because it runs at a different
/// statement position — after the index name conflict and the `IF NOT EXISTS` skip,
/// beside [`validate_btree_key_type`]. See [`validate_stored_expr_renders`].
fn validate_index_exprs_render(key: &IndexKey, predicate: Option<&Expr>) -> Result<()> {
    for expr in index_stored_exprs(key, predicate) {
        validate_stored_expr_renders(expr)?;
    }
    Ok(())
}

/// Build the storage `IndexSpec` for a catalog `index` over `table`, mapping an
/// unresolvable plain key column to a loud `ColumnNotFound`.
///
/// Thin wrapper over `index_spec_from_catalog` (the single catalog → storage
/// hand-off): the `index_spec_from_catalog` `None` (a plain key column missing
/// from `table`) can't happen on the `CREATE INDEX` paths — they validate columns
/// first — so it maps to a `ColumnNotFound` rather than silently dropping the index.
fn index_spec(
    index: &Index,
    table: &Table,
    enums: &BTreeMap<String, EnumType>,
) -> Result<IndexSpec> {
    index_spec_from_catalog(index, table, enums)?.ok_or_else(|| {
        crate::CatalogError::ColumnNotFound {
            table: index.table_name.clone(),
            // `None` only happens for a plain (bare-column) key whose column is
            // missing, so `bare_columns` is `Some` on this path.
            column: index
                .key
                .bare_columns()
                .map_or_else(|| "<expression key>".to_string(), |c| c.join(", ")),
        }
        .into()
    })
}

/// The single bare key column's type, if this is a plain single-column index —
/// `None` for a composite or expression key, which neither method-gate reads.
fn single_key_column_type(key: &IndexKey, table: &Table) -> Option<TypeOid> {
    key.single_bare_column().and_then(|only| {
        table
            .columns
            .iter()
            .find(|c| c.name == only)
            .map(|c| c.type_oid)
    })
}

/// Validate a `CREATE INDEX`'s declared **GIN** access method — the half of the
/// method check Postgres also raises **ahead of the index name**, so it runs there.
///
/// GIN requires a single bare column whose type is an array or JSONB, and is never
/// unique. Both halves are Postgres's own and both precede `index_create`'s name
/// conflict: `amcanunique` at `indexcmds.c:870` and `ResolveOpClass`'s *"data type
/// integer has no default operator class for access method gin"* (`indexcmds.c:2293`,
/// its `LOCATION`), reached inside `ComputeIndexAttrs`. Measured on 18.6:
/// `CREATE INDEX <a taken name> ON t USING gin (int_col)` is `42704`, **not** the name's
/// `42P07` — the opclass wins.
///
/// The B-tree side is deliberately *not* here; see [`validate_btree_key_type`].
///
/// **This gate reads the *column's* type and never an array's *element* type.**
/// `gin_decompose` hands an array's elements back verbatim and storage encodes each
/// with the *same* single-column `encode_probe_key` a B-tree key uses, so a GIN entry
/// is byte-for-byte an ordinary index entry, keyed by the element type's own arm of
/// `Value::encode_index_key` — a `jsonb[]`'s by the `jsonb` arm, so its probe matches by
/// `jsonb`'s equality. A JSONB **column**'s decomposition is `Value::Text` tokens.
///
/// # Errors
///
/// Returns a loud error when GIN and the column type disagree, or when a unique GIN
/// is asked for.
fn validate_index_method(
    method: IndexMethod,
    unique: bool,
    key: &IndexKey,
    table: &Table,
) -> Result<()> {
    let IndexMethod::Gin = method else {
        return Ok(());
    };
    let Some(col_type) = single_key_column_type(key, table) else {
        return Err(Error::NotImplemented(
            "GIN index requires a single bare array or JSONB column".into(),
        ));
    };
    if !col_type.is_container() {
        return Err(Error::NotImplemented(format!(
            "GIN index requires an array or JSONB column, not {col_type:?}"
        )));
    }
    if unique {
        return Err(Error::NotImplemented(
            "access method \"gin\" does not support unique indexes".into(),
        ));
    }
    Ok(())
}

/// Refuse the one index shape whose **declared** method is B-tree and whose **derived**
/// method is GIN: a non-unique `CREATE INDEX [USING btree]` over a single bare `jsonb`
/// or array column — a **pglet-only** restriction, which is why it runs *after* every
/// check Postgres has, the index name conflict and the `IF NOT EXISTS` skip included.
///
/// Postgres raises nothing here: `jsonb` has a default B-tree opclass (`jsonb_ops`)
/// and arrays have `array_ops`, so `CREATE INDEX ON t (jsonb_col)` builds a B-tree
/// there. pglet's key encoder has both families too (`Value::encode_index_key`), and
/// every other container key shape is built as the B-tree it declares — a unique one,
/// a composite, an expression key. What this shape lacks is a place to **record** the
/// method: the catalog `Index` carries none, [`index_method_for`] derives GIN for
/// exactly this shape, and a declared B-tree would be built, rendered and selected as
/// the GIN index it is not (`EDGES.md` row 25). So it is refused, loud, at the DDL —
/// with or without a partial predicate, which the derivation does not read.
///
/// A gate standing in for *no* Postgres error must not outrank a Postgres error, and
/// must certainly not outrank a Postgres **success**. Sequenced with the rest of the
/// method check it did exactly that: measured on 18.6, `CREATE INDEX IF NOT EXISTS
/// <a taken name> ON t (jsonb_col)` is a `NOTICE` + skip (a **success**), and pglet
/// answered `0A000`. The plain spelling was `42P07` there and `0A000` here.
///
/// The predicate is [`index_method_for`]'s own answer rather than a second reading of
/// the key, so the two cannot drift: whatever shape that function derives GIN for is
/// the shape a declared B-tree is refused over. The other direction — a declared
/// `gin` that would derive B-tree (a unique key, a non-container column, a composite
/// or expression key) — is [`validate_index_method`]'s.
///
/// # Errors
///
/// [`Error::UnindexableType`] naming the key column's type.
fn validate_btree_key_type(method: IndexMethod, index: &Index, table: &Table) -> Result<()> {
    if method == IndexMethod::Btree
        && index_method_for(index, table) == IndexMethod::Gin
        && let Some(col_type) = single_key_column_type(&index.key, table)
    {
        // The type name is SQL's, `format_type`'s spelling; the fallback is the
        // `Debug` form only for a type the deparser cannot name, which is none of
        // the container types this arm sees.
        let name =
            crate::deparse::catalog_type_name(col_type).unwrap_or_else(|_| format!("{col_type:?}"));
        return Err(Error::UnindexableType(name));
    }
    Ok(())
}

/// The one error a storage call whose other errors a DDL statement ignores must still
/// pass: a write the device refused, or a store refusing work since one
/// (`Error::WriteRefused` / `Error::DurableWriteFailed` /
/// `Error::DurableWriteUnconfirmed` / `Error::DatabaseMustBeReopened`). The statement
/// answers that rather than completing over a write that did not happen.
fn durable_write_failure(result: Result<()>) -> Result<()> {
    match result {
        Err(
            e @ (Error::WriteRefused { .. }
            | Error::DurableWriteFailed { .. }
            | Error::DurableWriteUnconfirmed { .. }
            | Error::DatabaseMustBeReopened(_)),
        ) => Err(e),
        _ => Ok(()),
    }
}

/// Execute CREATE TABLE.
///
/// After the table lands, each `PRIMARY KEY` / `UNIQUE` constraint in
/// `unique_constraints` (single-column or composite) becomes an auto-created
/// **enforced** unique index (`<table>_pkey` for PK, `<table>_<col…>_key` for
/// UNIQUE), persisted into `schema.indexes` and built in storage so it
/// re-registers on reopen and probes at write time (see `enforce_index_entries`).
///
/// `foreign_keys` (single- or multi-column) are validated + resolved **read-only,
/// before any schema mutation** (`resolve_and_validate_new_fk`): the referenced
/// columns must be covered by a PK/UNIQUE key on the parent (`42830` otherwise) — a
/// self-reference matches against the in-flight columns / unique constraints — so a
/// bad FK aborts the CREATE with the catalog untouched (no phantom half-created
/// table). They then ride on the new `Table` and are enforced at write/delete time
/// (`enforce_foreign_keys` / `execute_delete`).
///
/// **Everything fallible runs before `schema.create_table`**, and that is a
/// correctness property of the *statement*, not a tidiness preference: the
/// autocommit path rolls back **storage** on an error and leaves `self.schema`
/// mutated in place (`pglet`'s `run_planned_env`), and the next successful write
/// serializes the catalog, so a half-created table is **durable**. A `PRIMARY KEY (b)`
/// naming no column raised its `42703` from inside the index-build loop, by which point
/// `ph` was in the catalog: `information_schema.tables` listed it, `SELECT`/`INSERT`
/// answered `42P01` (storage had been rolled back), and `CREATE TABLE ph …` was `42P07`
/// forever.
///
/// The one check still standing after the mutation is `create_unique_index`'s
/// relation-name backstop, and every route into it has now been closed *upstream* by
/// making the namer read `catalog ∪ in-flight`: the table's own name, the before-list's
/// implicit sequences, and a user's `DEFAULT nextval('x')`. That last one was found two
/// passes later than the others and is the generalizable shape — **a name this statement
/// will register that a later gate cannot see**. Ask it of anything new this function
/// registers; a catalog-reading gate is blind to all of them by construction.
#[allow(clippy::too_many_arguments)]
pub fn execute_create_table<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    name: &str,
    columns: &[crate::planner::ColumnDef],
    checks: &[CheckConstraintDef],
    unique_constraints: &[UniqueConstraint],
    foreign_keys: &[ForeignKeyDef],
    if_not_exists: bool,
) -> Result<QueryResult> {
    // The name is checked against the whole **relation namespace**, not just the
    // tables: an index shares it, so `CREATE TABLE t_idx` after `CREATE INDEX t_idx …`
    // is `42P07` in Postgres (`heap_create_with_catalog`), and `IF NOT EXISTS` skips
    // on an index of that name too (measured on 18.6).
    //
    // `IF NOT EXISTS` short-circuits **before** everything below, because Postgres
    // tests it at the top of parse analysis (`transformCreateStmt`, right after
    // `RangeVarGetAndCheckCreationNamespace`) while the column and index-constraint
    // transforms run later in the same pass. Measured on 18.6: `CREATE TABLE IF NOT
    // EXISTS ex (a INT, PRIMARY KEY (nosuch))` against an existing `ex` is a plain
    // skip, not `42703`.
    if if_not_exists && schema.relation_name_taken(name) {
        return Ok(QueryResult::ddl("CREATE TABLE"));
    }

    // Convert planner ColumnDef to catalog Column. Type resolution fails loud on
    // an unsupported/unrecognized type (unless `coerce_unknown_types` is set), so
    // the collect is fallible.
    //
    // This runs **first** because Postgres resolves each column's type in the
    // `tableElts` walk (`transformColumnDefinition` → `transformColumnType`), which
    // precedes `transformIndexConstraints` in the same parse-analysis pass — so
    // `42704` outranks both the index-constraint errors below and the `42P07` for an
    // existing relation. Measured on 18.6: `CREATE TABLE x (a nosuchtype, PRIMARY KEY
    // (a), PRIMARY KEY (a))` is `42704`, not `42P16`.
    let mut table_columns: Vec<Column> = columns
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let rt = resolve_column_type(&c.type_name, schema)?;
            if c.identity.is_some() {
                ensure_identity_integer(rt.type_oid)?;
            }
            Ok(Column {
                name: c.name.clone(),
                type_oid: rt.type_oid,
                nullable: c.nullable,
                position: (i + 1) as i16,
                default: c.default.clone(),
                enum_type: rt.enum_type,
                numeric_typmod: rt.numeric_typmod,
                varchar_length: rt.varchar_length,
                temporal_precision: rt.temporal_precision,
                identity: c.identity,
            })
        })
        .collect::<Result<Vec<_>>>()?;

    // `transformIndexConstraints`: Postgres resolves the PK/UNIQUE constraints during
    // **parse analysis**, before the statement executes at all, so both errors this
    // loop raises outrank every execution-time error in the same statement —
    // the `42P07` just below for an existing relation, a duplicate CHECK name
    // (`42710`), an ill-typed CHECK (`42883`), and a foreign key to a missing parent
    // (`42P01`). All four measured on 18.6.
    //
    // The two checks are **interleaved per constraint**, not run as two whole-list
    // passes, because `transformIndexConstraint` tests the multiplicity at the top of
    // the function and resolves the key columns further down — inside the *same* call.
    // So which one answers depends on where each problem sits: `PRIMARY KEY (nosuch),
    // PRIMARY KEY (a)` is `42703` (constraint 1's columns are resolved before
    // constraint 2 is ever reached) while `PRIMARY KEY (a), PRIMARY KEY (nosuch)` is
    // `42P16` (constraint 2's multiplicity is tested before its own columns). Neither
    // follows from the other and no whole-list ordering yields both — the same shape
    // as the CHECK cook-then-name loop one section down.
    //
    // Two PKs are rejected even when identical; only the *non-PK* redundancy is merged
    // (`dedup_unique_constraints`, which runs after — the same order as
    // `transformIndexConstraint` raising before its caller's dedup loop).
    //
    // The key-list walk is interleaved one scale further down too, because the
    // repeated-column check sits *below* the resolution inside the same per-key loop
    // body: a key is resolved, then tested against the keys already accepted. So
    // `PRIMARY KEY (a, a, nope)` is `42701` and `PRIMARY KEY (a, nope, a)` is `42703`
    // — measured on 18.6, and neither a whole-list resolve-then-dedup nor its reverse
    // yields both. [`validate_key_columns`] owns that body; the `ALTER TABLE …
    // ADD CONSTRAINT` route calls the same function with the resolution switched off,
    // which is what 18.6's `!cxt->isalter` does.
    //
    // All of these checks must land here, ahead of `schema.create_table`, or the
    // failure is a durable phantom table (see this function's doc comment).
    let mut primary_key_seen = false;
    for uc in unique_constraints {
        if uc.is_primary {
            if primary_key_seen {
                return Err(crate::CatalogError::MultiplePrimaryKeys {
                    table: name.to_string(),
                }
                .into());
            }
            primary_key_seen = true;
        }
        validate_key_columns(name, Some(&table_columns), uc)?;
    }
    let unique_constraints = &dedup_unique_constraints(unique_constraints)[..];

    // The implicit sequences a SERIAL / identity column brings, **named here** rather
    // than at parse time — a declared `SEQUENCE NAME <n>` verbatim, an engine-chosen
    // one disambiguated against the live relation namespace
    // (`resolve_implicit_sequence_name`).
    //
    // This runs before the constraint names, and the order is observable: Postgres
    // puts the serial/identity `CreateSeqStmt`s in `transformCreateStmt`'s
    // *before*-list, so the sequences exist by the time `index_create` picks a
    // constraint index's name. Measured on 18.6: `CREATE TABLE b1 (a SERIAL,
    // CONSTRAINT b1_a_seq PRIMARY KEY (a))` is `42P07` on `b1_a_seq` — the sequence
    // took the name first. So the names claimed here are seeded into `ConstraintNames`
    // below.
    //
    // The same before-list also puts it ahead of the table's **own** `42P07`, which is
    // why this block sits above that check rather than below it: with a table `z1` and
    // a sequence `z2` both present, `CREATE TABLE z1 (id INT GENERATED ALWAYS AS
    // IDENTITY (SEQUENCE NAME z2))` names `z2`, not `z1` (measured on 18.6). It stays
    // *below* the type resolution and the key-column/multiplicity loop, which run in
    // parse analysis and outrank the whole before-list (`42704` and `42703` both
    // measured to win over a colliding `SEQUENCE NAME`).
    //
    // Everything here is read-only; the catalog is untouched until the sequences are
    // inserted with the table.
    let mut claimed_relations: HashSet<String> = HashSet::new();
    let mut new_sequences: Vec<(String, Sequence)> = Vec::new();
    for (i, c) in columns.iter().enumerate() {
        if c.implicit_sequence {
            let seq = resolve_implicit_sequence_name(
                schema,
                &claimed_relations,
                name,
                &c.name,
                c.sequence_name.as_deref(),
            )?;
            claimed_relations.insert(seq.clone());
            // An identity column's `START WITH n` seeds the sequence at `n`; a plain
            // SERIAL starts at 1. The sequence is owned by the column it feeds.
            let start = c.identity_start.unwrap_or(1);
            new_sequences.push((seq.clone(), Sequence::owned_by(start, name, &c.name)));
            table_columns[i].default = Some(ColumnDefault::NextVal(seq));
        }
    }

    // The table's own name, against the catalog **∪ the before-list this statement
    // just spoke for** — the second half is what makes a declared `SEQUENCE NAME`
    // equal to the table's own name an error, and it is why the check lands *here*
    // rather than inside the loop above. Postgres creates the sequence first and
    // finds the collision in `heap_create_with_catalog`, so the message names the
    // **table**: `CREATE TABLE a1 (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME
    // a1))` is `42P07 relation "a1" already exists` on 18.6 (and under `IF NOT
    // EXISTS` too, whose skip only fires when the table itself already exists).
    // Seeding the table's name into the loop instead would answer the same SQLSTATE
    // but name the wrong relation and, worse, at the wrong *position*: with a second
    // identity column declaring an already-taken name, 18.6 names **that** one
    // (measured in both written orders), because the whole before-list runs before
    // the heap. The catalog cannot hold the table yet — nothing is mutated until
    // every check below has passed — so the in-flight set is the only way this gate
    // sees it, the same reason `ConstraintNames::new` seeds it one block down.
    //
    // **A relation's column names are a set**, and this is where the two halves of
    // that land — below the before-list, above the table's own `42P07`. Both are
    // 18.6 errors with a measured position, so each goes where 18.6 raises it rather
    // than merely early:
    //
    // - `MergeAttributes` walks `stmt->tableElts` in `DefineRelation`, after the
    //   whole parse-analysis pass, so a repeat loses to the column types (`42704` and
    //   `22023` measured — and to any other verdict of the one `resolve_column_type`
    //   call above, which is the whole of that family here), to the index constraints
    //   (`42P16`, the key columns' `42703`/`42701`) and to a colliding declared
    //   `SEQUENCE NAME` (`42P07`) — and beats the table's own `42P07` and the
    //   column-default cook below.
    // - `CheckAttributeNamesTypes` runs one layer further in, inside
    //   `heap_create_with_catalog`, which is why a system column name loses to the
    //   repeat: `CREATE TABLE d (ctid int, a int, a int)` is *specified more than
    //   once* on 18.6 in either written order, measured.
    //
    // The names are folded where the parser read them, so both tests are plain byte
    // comparisons: unquoted `A` collides with `"a"` and an unquoted `CTID` is
    // refused, while `"a"` beside `"A"` is two columns and `"CTID"` is an ordinary
    // one — each measured on both engines. The two refusals are pinned by fixtures
    // that write the unquoted name in **upper** case
    // (`ddl/create_table_duplicate_column_name_folds_the_unquoted_spelling.sql`,
    // `ddl/create_table_system_column_name_folds_the_unquoted_spelling.sql`); a
    // fixture spelling it already-lower-case exercises no fold and stands in for
    // neither.
    for (i, c) in table_columns.iter().enumerate() {
        if table_columns[..i].iter().any(|prev| prev.name == c.name) {
            return Err(crate::CatalogError::DuplicateColumnName {
                column: c.name.clone(),
            }
            .into());
        }
    }
    for c in &table_columns {
        if is_system_column_name(&c.name) {
            return Err(crate::CatalogError::SystemColumnName {
                column: c.name.clone(),
            }
            .into());
        }
    }

    if schema.relation_name_taken(name) || claimed_relations.contains(name) {
        return Err(crate::CatalogError::TableAlreadyExists {
            table: name.to_string(),
        }
        .into());
    }
    // The table's row type, against the **type** namespace, directly behind the
    // relation name — the order `heap_create_with_catalog` tests the two in, so it too
    // loses to everything above and beats everything below: measured on 18.6, a table
    // over an enum's name with a missing FK parent, or with a constraint index named
    // like an existing relation, is `42710`, and with a column of no such type it is
    // `42704`. `IF NOT EXISTS` does not reach it — the skip above reads the relation
    // namespace only — so `CREATE TABLE IF NOT EXISTS <an enum's name>` is `42710` too.
    if schema.type_name_taken(name) {
        return Err(Error::RelationTypeAlreadyExists(name.to_string()));
    }

    // A user's own `DEFAULT nextval('x')` — a *cooked default*, not a before-list
    // entry, so it is resolved in its own pass **after** the table's `42P07` rather
    // than interleaved with the sequences above. Postgres cooks column defaults in
    // `AddRelationNewConstraints`, once the heap exists, and that position is measured
    // from both sides on 18.6 with a default naming a sequence that does not exist: its
    // `42P01` **loses** to the column type (`42704`), the key-list pass (the key-column
    // `42703`, a repeated key column's `42701`, the multiplicity `42P16`), the
    // relation's column-name set (a duplicate and a system-owned name, each `42701`)
    // and the table's own `42P07`, and **beats** a duplicate CHECK name (`42710`), an
    // ill-typed CHECK (`42883`), a declared constraint-index name already taken
    // (`42P07`), a missing FK parent (`42P01`) and a repeated column in a foreign key's
    // *referenced* list (`42830`). A colliding
    // `SEQUENCE NAME` on a *later* column also beats it (`CREATE TABLE d3 (b INT DEFAULT
    // nextval('d3'), a INT GENERATED … (SEQUENCE NAME <taken>))` names `<taken>`),
    // because the whole before-list precedes the heap; and `'x'::regclass` resolving to
    // the table being created is simply the table (`nextval('e3')` inside `CREATE TABLE
    // e3` binds the *table*).
    //
    // The pass split is also what keeps the *records* right, and that half was a
    // silent-wrong rather than an error-order nicety: `CREATE TABLE k4 (b INT DEFAULT
    // nextval('k4_a_seq'), a INT GENERATED ALWAYS AS IDENTITY (START WITH 100))` is
    // accepted by both engines — the before-list has already created `k4_a_seq` when the
    // default is cooked, so the two columns share it — and answers 100 on 18.6. Under one
    // interleaved loop pglet registered a bare `Sequence::new(1)` for column `b` first,
    // which won the `or_insert` below and discarded the identity column's `START WITH`
    // *and* its ownership edge; it answered 1, and the dump lost the `OWNED BY`.
    //
    // The name is never renamed, and registered only if nothing already holds it — an
    // existing sequence keeps its counter, its start and its owner. (This default creates
    // no ownership edge of its own; Postgres records an `attrdef → sequence` edge
    // instead, which pglet does not model.) The `claimed_relations` test is what dedups
    // a name two columns share, in either direction: against a sequence pass 1 already
    // spoke for, and against an earlier column of this same pass.
    //
    // The name is then **claimed**, because this statement is going to register it and
    // the constraint indexes named one block down must see it — Postgres's `pg_class`
    // already holds the sequence there, since a `DEFAULT nextval('x')` it accepts names
    // a relation that exists. Without the claim the auto-name is minted *on top of* the
    // sequence and only `create_unique_index`'s post-mutation backstop catches it: the
    // `CREATE TABLE` half burned the table name (durably, on the autocommit path) and the
    // `ADD COLUMN` half left a populated table whose rows are one value short of its
    // catalog columns, so every `SELECT *` died at the wire. With the claim both routes
    // disambiguate to `<name>1`, byte-identical to 18.6 with the sequence pre-created.
    // The defaults themselves are cooked **here**, in declared column order, and the
    // position is measured on 18.6 from both sides (see `cook_column_default`): the
    // verdict loses to the column type, the key-list pass above (`42P16`, a key
    // column's `42703`, a repeated key column's `42701`), a colliding declared
    // `SEQUENCE NAME`, a duplicate column, a system-owned column name and the table's
    // own `42P07` — all above — and beats the constraint **naming** block below. Read
    // "the constraint block" as that block and not as every constraint gate: a
    // `PRIMARY KEY`/`UNIQUE` key list is validated in the pass above, so
    // `PRIMARY KEY (a, a)` beside a bad default is `42701` rather than the cook's
    // verdict, on both engines. An `implicit_sequence` column has no user
    // default to cook (the parser refuses `SERIAL DEFAULT 5` as
    // `42601 multiple default values`, as 18.6 does), so it is skipped rather than
    // probed against a sequence that does not exist yet.
    for (i, c) in columns.iter().enumerate() {
        if c.implicit_sequence {
            continue;
        }
        if let Some(default) = &c.default {
            cook_column_default(default, &table_columns[i], DefaultTiming::Deferred, schema)?;
        }
    }

    // **Every sequence the default draws from**, through the one derivation
    // (`default_sequence_refs`) the `DROP` edge and the dump's hoist already share —
    // not the closed `NextVal` arm alone. A general expression default names a
    // sequence exactly as the bare spelling does, and a registration that could not
    // see it left the whole family half-implemented in one direction each:
    // `DEFAULT nextval('nosuch') + 1` registered nothing and then answered `42P01` at
    // every write (the bare spelling auto-mints), `nextval('<a table>') + 1` skipped
    // the kind gate below, and `nextval('t1_pkey') + 1` claimed nothing so the
    // constraint index was minted straight onto the name the sequence would take —
    // the accept-then-brick shape the claim exists to stop. Measured on 18.6, a
    // multi-sequence default (`nextval('a') + nextval('b')`) records a `pg_attrdef`
    // dependency on **both**, which is why this is a list rather than an `Option`.
    let mut user_default_sequences: Vec<String> = Vec::new();
    for c in columns {
        if c.implicit_sequence {
            continue;
        }
        if let Some(default) = &c.default {
            for seq in default_sequence_refs(default) {
                if !schema.sequences.contains_key(&seq) && !claimed_relations.contains(&seq) {
                    new_sequences.push((seq.clone(), Sequence::new(1)));
                }
                claimed_relations.insert(seq.clone());
                user_default_sequences.push(seq);
            }
        }
    }

    // Constraint names, in Postgres's own resolution order (measured on 18.6), through
    // the one gate that reads both namespaces (`ConstraintNames`). The CHECKs are
    // attached as the heap is created, so a duplicate CHECK name is reported before any
    // FK is looked at; then each PK/UNIQUE index is built (a taken *relation* name is
    // `42P07`, a name already used by one of those CHECKs is `42710`); then each FK is
    // added in source order, its name checked **before** its parent relation is
    // resolved — so a duplicate name on the *second* FK wins over that FK's missing
    // parent table, while a missing parent on the *first* one wins over the duplicate.
    // All of it runs read-only, before any mutation.
    //
    // The names each kind claims are visible to the next: `CONSTRAINT n CHECK (…),
    // CONSTRAINT n PRIMARY KEY (…)` is `42710` in either written order, and a PK/UNIQUE
    // name blocks a later FK of the same name. Per-kind pools would let every cross-kind
    // pair through.
    let mut names = ConstraintNames::new(schema, name);
    names.seed_relations(claimed_relations);

    // Type-check each CHECK expression against the new columns **before any mutation**,
    // so `CHECK (int_col LIKE '1%')` is a `42883` on this statement (as in Postgres)
    // instead of an accepted constraint that errors on every insert — and **interleaved**
    // with the naming, one constraint at a time, because that is what
    // `AddRelationNewConstraints` does (`heap.c`: cook, then name, per `cdef`). Two
    // whole-list passes cannot reproduce it: with all the expressions checked first,
    // `CONSTRAINT n CHECK (a > 0), CONSTRAINT n CHECK (a > 1), CONSTRAINT m CHECK (a
    // LIKE 'x')` answers `42883` where Postgres answers `42710` — constraint 2's *name*
    // beats constraint 3's *expression*, and the reverse arrangement still answers
    // `42883` on both. The same source-order rule as everywhere else, one scale down.
    //
    // Postgres words the CHECK-vs-CHECK collision `check constraint "<n>" already
    // exists` (`heap.c`) rather than the relation-scoped spelling; same `42710`.
    let named_checks: Vec<CheckConstraint> = checks
        .iter()
        .map(|check| {
            validate_check_exprs(std::slice::from_ref(check), &table_columns, &schema.enums)?;
            let name = names.mint_check(check)?;
            Ok(CheckConstraint {
                name,
                expr: check.expr.clone(),
            })
        })
        .collect::<Result<Vec<_>>>()?;

    let index_names: Vec<String> = unique_constraints
        .iter()
        .map(|uc| names.mint_constraint_index(uc))
        .collect::<Result<Vec<_>>>()?;

    // Validate + resolve every FK **read-only, before any schema mutation**, so a bad
    // FK (missing parent `42P01` / no covering unique key `42830` / arity mismatch /
    // column types 18.6 cannot compare `42804`) aborts the CREATE with the catalog
    // untouched — no phantom half-created table left on the autocommit path (which does
    // not restore the schema on error). A self-referential FK validates against the
    // in-flight columns / unique constraints (its parent — this table — isn't in the
    // schema yet). Mirrors the validate-before-mutate discipline of `add_foreign_key`.
    //
    // One FK at a time — name, resolution, types — because 18.6 adds them one
    // `ALTER TABLE` subcommand at a time after the heap and its indexes exist: the first
    // FK's `42804` outranks a later FK's duplicate name (`42710`) or missing parent
    // (`42P01`), and loses to a CHECK's or an index name's error wherever either is
    // written.
    let mut resolved_fks: Vec<ForeignKey> = Vec::with_capacity(foreign_keys.len());
    for fk in foreign_keys {
        let fk_name = names.mint_foreign_key(fk)?;
        let resolved = resolve_and_validate_new_fk(
            schema,
            name,
            &table_columns,
            unique_constraints,
            fk,
            fk_name,
        )?;
        let referenced = if resolved.parent_table == name {
            &table_columns
        } else {
            &get_table_or_err(schema, &resolved.parent_table)?.columns
        };
        validate_foreign_key_types(&resolved, &table_columns, referenced)?;
        resolved_fks.push(resolved);
    }

    // The kind gate on those user defaults, **last** — after every check Postgres
    // performs at DDL time, and still before any mutation. It is a deliberate
    // over-rejection standing in for an error Postgres raises at the *first write*
    // (`init_sequence`'s `validate_relation_kind`), so it must not outrank an error
    // Postgres raises at DDL: measured on 18.6, `CREATE TABLE c (b INT DEFAULT
    // nextval('<a table>'), CONSTRAINT n CHECK …, CONSTRAINT n CHECK …)` is `42710` and
    // the same default with `REFERENCES nosuchparent(q)` is `42P01`, because the cook
    // step itself succeeds there — only the write fails. Gating inside the pass above
    // answered `42809` to both. See `ensure_sequence_relation`.
    for seq in &user_default_sequences {
        ensure_sequence_relation_for_new_table(schema, name, seq)?;
    }

    // The render probe on every CHECK, in one pass over the whole statement and
    // **last** — the other pglet-only over-rejection on this route, and behind the
    // kind gate above only because neither is observable against 18.6, which accepts
    // both families. Everything above it is a gate standing in for an error 18.6
    // really raises, and riding inside the cook loop it pre-empted three of them in
    // both source orders (a taken constraint-index name's `42P07`, a missing FK
    // parent's `42P01`) or in one (a later duplicate CHECK name's `42710`) — see
    // `validate_stored_expr_renders`. Still before `generate_object_oid`, which
    // bumps the persisted `next_oid` and is this statement's first catalog write.
    validate_check_exprs_render(&named_checks)?;

    // Generate a unique OID for this table
    let oid = generate_object_oid(schema);

    let table = Table {
        name: name.to_string(),
        schema: "public".to_string(),
        columns: table_columns,
        oid,
        // Every CHECK carries the name minted above — the one fact the write-path
        // violation message, `information_schema`, and the dump all read.
        checks: named_checks,
        // FKs were validated + resolved read-only above (self-references included).
        foreign_keys: resolved_fks,
    };

    schema.create_table(table);
    // The sequences named above. `or_insert` is a no-op for the minted names (free by
    // construction) and preserves an existing sequence a `DEFAULT nextval('x')` names.
    for (seq, def) in new_sequences {
        schema.sequences.entry(seq).or_insert(def);
    }
    // Also create in storage (ignore errors if storage doesn't support it)
    durable_write_failure(storage.create_table(name))?;

    // Create one enforced unique index per PK/UNIQUE constraint (single-column or
    // composite), under the name minted and gated above — never re-derived here, or the
    // disambiguation and the gate would answer differently. These are plain
    // predicate-free enforced unique indexes, so they probe at write time — and they
    // carry the constraint marker, so `DROP INDEX` refuses them.
    for (uc, index_name) in unique_constraints.iter().zip(&index_names) {
        create_unique_index(
            schema,
            storage,
            index_name,
            name,
            &uc.columns,
            constraint_kind_of(uc),
        )?;
    }

    Ok(QueryResult::ddl("CREATE TABLE"))
}

/// Whether two column-name lists denote the same SET (order-independent) — the
/// covering-key match for a self-referential FK against the in-flight unique
/// constraints (the [`Schema::covering_unique_index`] equivalent for a table not yet
/// in the catalog).
fn column_sets_eq(a: &[String], b: &[String]) -> bool {
    a.len() == b.len() && a.iter().all(|c| b.contains(c))
}

/// Validate + resolve a foreign key declared on a table being CREATEd, **purely
/// read-only** (no schema mutation), returning the catalog FK under the already-minted
/// `name` and with `parent_columns` filled in — so a bad FK aborts the CREATE with the
/// catalog untouched.
///
/// The child columns are validated against the in-flight `new_columns` and the
/// referenced key against `schema`, **except a self-referential FK** whose parent is
/// the table being created: its columns and PK/UNIQUE keys aren't in `schema` yet, so
/// they're taken from `new_columns` / `new_uniques`. The verdicts come in 18.6's order
/// (`ATAddForeignKeyConstraint`) — the parent relation, then the referencing columns,
/// then the referenced ones, then the referenced list's repeat — and then the arity
/// ahead of the covering key, the reverse of 18.6's, which no statement observes: the
/// arity this reaches is an omitted list's, resolved to a primary key that covers
/// itself, since a written list's is the parser's, refused before any of this
/// (`EDGES.md` row 275). [`resolve_and_validate_foreign_key`] keeps the same order. The
/// column types are not compared here: [`validate_foreign_key_types`] is its callers'
/// next step.
///
/// # Errors
///
/// `CatalogError::TableNotFound` / `Error::WrongRelationKind` (parent — the variant
/// `get_table_or_err` returns, which is `validate_relation_kind`'s wording and *not*
/// `Error::WrongObjectType`'s, though the two share `42809`),
/// `Error::ForeignKeyColumnNotFound` (a column either list names that its table does
/// not have), `Error::ForeignKeyDuplicateReferencedColumn` (a repeated referenced
/// column), `Error::ForeignKeyColumnMismatch`, or `Error::NoUniqueForForeignKey`.
fn resolve_and_validate_new_fk(
    schema: &Schema,
    new_table: &str,
    new_columns: &[Column],
    new_uniques: &[UniqueConstraint],
    fk: &ForeignKeyDef,
    name: String,
) -> Result<ForeignKey> {
    let self_ref = fk.parent_table == new_table;
    // The parent must exist (`42P01`) *and* be a table: Postgres opens the referenced
    // relation first and answers `42809` (`referenced relation "s" is not a table`) for
    // a sequence — ahead of a referencing column the new table lacks.
    let parent_columns_in_catalog: Option<&[Column]> = if self_ref {
        None
    } else {
        Some(&get_table_or_err(schema, &fk.parent_table)?.columns)
    };
    // Child columns must exist on the new table (`42703`).
    validate_foreign_key_columns_exist(new_columns, &fk.columns)?;
    // Referenced columns: the explicit list, else the parent's PRIMARY KEY (from the
    // in-flight unique constraints for a self-reference, else the catalog PK index).
    let parent_columns: Vec<String> = if !fk.parent_columns.is_empty() {
        fk.parent_columns.clone()
    } else if self_ref {
        new_uniques
            .iter()
            .find(|uc| uc.is_primary)
            .map(|uc| uc.columns.clone())
            .ok_or_else(|| Error::NoUniqueForForeignKey {
                table: fk.parent_table.clone(),
            })?
    } else {
        schema
            .primary_key_index(&fk.parent_table)
            .and_then(|pk| pk.key.bare_columns().map(<[String]>::to_vec))
            .ok_or_else(|| Error::NoUniqueForForeignKey {
                table: fk.parent_table.clone(),
            })?
    };
    validate_foreign_key_columns_exist(
        parent_columns_in_catalog.unwrap_or(new_columns),
        &parent_columns,
    )?;
    validate_referenced_columns_distinct(&parent_columns)?;
    if parent_columns.len() != fk.columns.len() {
        return Err(Error::ForeignKeyColumnMismatch);
    }
    // A covering unique key (set-match) must exist on the parent.
    let covered = if self_ref {
        new_uniques
            .iter()
            .any(|uc| column_sets_eq(&uc.columns, &parent_columns))
    } else {
        schema
            .covering_unique_index(&fk.parent_table, &parent_columns)
            .is_some()
    };
    if !covered {
        return Err(Error::NoUniqueForForeignKey {
            table: fk.parent_table.clone(),
        });
    }
    Ok(ForeignKey {
        name,
        columns: fk.columns.clone(),
        parent_table: fk.parent_table.clone(),
        parent_columns,
        on_delete: fk.on_delete,
        on_update: fk.on_update,
    })
}

/// Resolve a foreign key's referenced columns and verify the parent has a covering
/// unique key, returning the catalog FK under the already-minted `name` and with its
/// `parent_columns` filled in.
///
/// `referencing` is the column set the FK's own columns are looked up in — the child
/// table's, plus the added column on `ADD COLUMN`, which is not in the catalog yet —
/// and, for a self-reference (`fk.parent_table` is `child_table`), the referenced ones
/// too ([`foreign_key_referenced_columns`]). An omitted `REFERENCES parent` (empty `parent_columns`) resolves to the parent's
/// PRIMARY KEY. The referencing/referenced arity must agree (`ForeignKeyColumnMismatch`,
/// `42830`), and the referenced columns must be covered by a PK/UNIQUE key on the
/// parent, matched as an **order-independent set** (`Schema::covering_unique_index`);
/// no covering key is `NoUniqueForForeignKey` (`42830`). The FK's own local↔parent
/// pairing stays positional. The parent table must exist (`42P01`) and be a table
/// (`42809` for a sequence).
///
/// The verdicts come in `ATAddForeignKeyConstraint`'s order — the parent relation, the
/// referencing columns, the referenced columns, the referenced list's repeat — and then
/// the arity ahead of the covering key, the reverse of 18.6's and unobservable for the
/// reason [`resolve_and_validate_new_fk`] gives; a written list's arity mismatch never
/// reaches here (the parser refuses it first, `EDGES.md` row 275). The column types are
/// not compared here: [`validate_foreign_key_types`] is the caller's, at the position its
/// route gives it.
///
/// # Errors
///
/// `CatalogError::TableNotFound` (missing parent), `Error::WrongRelationKind` (a parent
/// name another relation kind holds — both come from `get_table_or_err`, and the
/// wrong-kind one is that helper's variant, not `Error::WrongObjectType`, which words
/// the same `42809` as a `DROP`/`TRUNCATE` does),
/// `Error::ForeignKeyColumnNotFound` (a column either list names that its table does
/// not have), `Error::ForeignKeyDuplicateReferencedColumn` (a
/// repeated referenced column), `Error::ForeignKeyColumnMismatch` (arity), or
/// `Error::NoUniqueForForeignKey` (no covering key).
fn resolve_and_validate_foreign_key(
    schema: &Schema,
    child_table: &str,
    fk: &ForeignKeyDef,
    name: String,
    referencing: &[Column],
) -> Result<ForeignKey> {
    // The parent must exist (`42P01`) *and* be a table — a sequence is `42809`
    // (`referenced relation "s" is not a table`), on this route too (measured for
    // `ALTER TABLE … ADD CONSTRAINT … REFERENCES <a sequence>`).
    let parent = get_table_or_err(schema, &fk.parent_table)?;
    validate_foreign_key_columns_exist(referencing, &fk.columns)?;
    // Referenced columns: the explicit list, else the parent's PRIMARY KEY.
    let parent_columns: Vec<String> = if fk.parent_columns.is_empty() {
        schema
            .primary_key_index(&fk.parent_table)
            .and_then(|pk| pk.key.bare_columns().map(<[String]>::to_vec))
            .ok_or_else(|| Error::NoUniqueForForeignKey {
                table: fk.parent_table.clone(),
            })?
    } else {
        fk.parent_columns.clone()
    };
    validate_foreign_key_columns_exist(
        foreign_key_referenced_columns(&parent.columns, &fk.parent_table, child_table, referencing),
        &parent_columns,
    )?;
    validate_referenced_columns_distinct(&parent_columns)?;
    // Arity must agree (the parser checks the explicit case; this also covers the
    // omitted-list case resolved to a differently-sized PK).
    if parent_columns.len() != fk.columns.len() {
        return Err(Error::ForeignKeyColumnMismatch);
    }
    // A covering unique key (set-match) must exist on the parent.
    if schema
        .covering_unique_index(&fk.parent_table, &parent_columns)
        .is_none()
    {
        return Err(Error::NoUniqueForForeignKey {
            table: fk.parent_table.clone(),
        });
    }
    Ok(ForeignKey {
        name,
        columns: fk.columns.clone(),
        parent_table: fk.parent_table.clone(),
        parent_columns,
        on_delete: fk.on_delete,
        on_update: fk.on_update,
    })
}

/// The column set a foreign key's **referenced** list resolves in on the `ALTER TABLE`
/// routes: the parent's catalog columns, or, for a self-reference (`parent_table` is
/// `child_table`), `referencing` — the set the referencing list resolves in, which on
/// `ADD COLUMN` holds the added column the catalog does not have yet. 18.6 adds the key
/// in the pass after the column's, so `ALTER TABLE s ADD COLUMN a text REFERENCES s (a)`
/// finds `a` and answers the covering key's `42830`, not a missing column's `42703`.
fn foreign_key_referenced_columns<'a>(
    parent_columns: &'a [Column],
    parent_table: &str,
    child_table: &str,
    referencing: &'a [Column],
) -> &'a [Column] {
    if parent_table == child_table {
        referencing
    } else {
        parent_columns
    }
}

/// Validate that every column a foreign key's list names is one of `table_columns`
/// (`42703`).
///
/// 18.6's `transformColumnNameList`, which words the refusal the same for both lists —
/// `column "x" referenced in foreign key constraint does not exist` — and raises it for
/// the referenced list **ahead of** the covering-key lookup, so a key naming a column
/// the parent lacks is never answered *there is no unique constraint matching given
/// keys*.
fn validate_foreign_key_columns_exist(table_columns: &[Column], columns: &[String]) -> Result<()> {
    if let Some(missing) = columns
        .iter()
        .find(|c| !table_columns.iter().any(|tc| &tc.name == *c))
    {
        return Err(Error::ForeignKeyColumnNotFound {
            column: missing.clone(),
        });
    }
    Ok(())
}

/// Refuse a resolved foreign key one of whose column pairs 18.6 cannot implement —
/// `42804 foreign key constraint "…" cannot be implemented`, naming the **first** such
/// pair in key order, under the constraint's minted name.
///
/// `referencing` and `referenced` are the column sets the two lists resolve in: the
/// child's and the parent's, which are the same set for a self-reference and, on
/// `CREATE TABLE`, the in-flight columns rather than the catalog's. The verdict is
/// [`crate::planner::foreign_key_types_compatible`]'s, over each column's type **with its
/// enum identity** — an enum column's storage OID is `text`, which would make `mood`
/// and `text` one type.
///
/// Where it runs is its route's decision, because 18.6 raises it in its own `ALTER
/// TABLE` pass: after the referenced key resolves on every route, after the statement's
/// `CHECK`s and constraint indexes on `CREATE TABLE`, before the existing rows are read
/// on `ADD CONSTRAINT`, and — on `ADD COLUMN` — before the rewrite's row checks but
/// after an index build that runs without a rewrite (`execute_alter_table_add_column`).
///
/// # Errors
///
/// `Error::ForeignKeyCannotBeImplemented`, or `Error::ForeignKeyColumnNotFound` for a
/// column the sets do not hold — which the resolvers above have already refused.
fn validate_foreign_key_types(
    fk: &ForeignKey,
    referencing: &[Column],
    referenced: &[Column],
) -> Result<()> {
    for (column, referenced_column) in fk.columns.iter().zip(&fk.parent_columns) {
        let child = foreign_key_column_type(referencing, column)?;
        let parent = foreign_key_column_type(referenced, referenced_column)?;
        if !crate::planner::foreign_key_types_compatible(&child, &parent) {
            return Err(Error::ForeignKeyCannotBeImplemented(Box::new(
                pglet_core::ForeignKeyTypeMismatch {
                    constraint: fk.name.clone(),
                    column: column.clone(),
                    referenced_column: referenced_column.clone(),
                    column_type: foreign_key_type_name(&child),
                    referenced_type: foreign_key_type_name(&parent),
                },
            )));
        }
    }
    Ok(())
}

/// The type of the column `name` in `columns`, with its enum identity.
fn foreign_key_column_type(columns: &[Column], name: &str) -> Result<crate::planner::ResolvedType> {
    columns
        .iter()
        .find(|c| c.name == name)
        .map(|c| crate::planner::ResolvedType::new(c.type_oid, c.enum_type.clone()))
        .ok_or_else(|| Error::ForeignKeyColumnNotFound {
            column: name.to_string(),
        })
}

/// A column type as 18.6's `format_type_be` prints it in the `42804` DETAIL — an enum
/// by its name through `quote_identifier`, everything else by its SQL name.
fn foreign_key_type_name(ty: &crate::planner::ResolvedType) -> String {
    match ty.enum_type.name() {
        Some(name) if ty.oid.array_element().is_some() => {
            format!("{}[]", crate::lexer::pg_quote::quote_identifier(name))
        }
        Some(name) => crate::lexer::pg_quote::quote_identifier(name),
        None => ty.display_name(),
    }
}

/// The [`ConstraintKind`] a [`UniqueConstraint`] declares.
const fn constraint_kind_of(uc: &UniqueConstraint) -> ConstraintKind {
    if uc.is_primary {
        ConstraintKind::PrimaryKey
    } else {
        ConstraintKind::Unique
    }
}

/// The two name spaces a table constraint occupies, gated together.
///
/// Postgres keeps constraint names in two places at once and pglet has to read both
/// or it under-rejects in one direction and over-rejects in the other:
///
/// - the **relation namespace** (`pg_class`: tables + indexes, per schema), which a
///   `PRIMARY KEY` / `UNIQUE` enters because its *index* is named after it — a
///   collision is `42P07`, `relation "<n>" already exists`;
/// - the **per-relation constraint namespace** (`pg_constraint (conrelid, conname)`,
///   one unique index over all four kinds), which every constraint enters — a
///   collision is `42710`.
///
/// A CHECK or FOREIGN KEY is only in the second; a PK/UNIQUE is in both, and
/// `index_create` tests them in that order (`index.c`, the relation check then the
/// constraint check), which is why [`Self::claim_constraint_index`] does too.
///
/// Both spaces are consulted as `catalog ∪ claimed`: the names this statement has
/// already handed out are not in the catalog yet (a `CREATE TABLE`'s table isn't
/// created until its constraints validate), and Postgres makes them visible to the
/// next constraint via `CommandCounterIncrement`. So one type serves the
/// `CREATE TABLE` path (empty catalog side for this table) and the `ALTER TABLE`
/// paths (empty claimed side) identically.
struct ConstraintNames<'a> {
    schema: &'a Schema,
    table: &'a str,
    /// Relation names this statement has minted but not yet inserted into
    /// `schema.indexes`.
    claimed_relations: HashSet<String>,
    /// Constraint names this statement has handed out but not yet recorded on the
    /// catalog table.
    claimed_constraints: HashSet<String>,
}

impl<'a> ConstraintNames<'a> {
    /// `table`'s own name is **seeded into the relation namespace**, because Postgres
    /// creates the heap (`heap_create_with_catalog`) before any of its constraint
    /// indexes, so by the time `index_create` picks a name the table is already in
    /// `pg_class`: `CREATE TABLE selfn (a INT, CONSTRAINT selfn PRIMARY KEY (a))` is
    /// `42P07` on 18.6. For the `ALTER` paths the catalog already holds it and the seed
    /// is a no-op — but on the `CREATE TABLE` path it is the difference between the
    /// error landing here, read-only, and landing in the index-build loop *after*
    /// `schema.create_table`, which the autocommit path makes a durable phantom.
    fn new(schema: &'a Schema, table: &'a str) -> Self {
        Self {
            schema,
            table,
            claimed_relations: HashSet::from([table.to_string()]),
            claimed_constraints: HashSet::new(),
        }
    }

    /// Add relation names this statement has already claimed outside the constraint
    /// machinery — the implicit sequences of a `CREATE TABLE`'s / `ADD COLUMN`'s
    /// SERIAL columns, which Postgres creates before the table's indexes exist.
    fn seed_relations(&mut self, names: HashSet<String>) {
        self.claimed_relations.extend(names);
    }

    /// Whether `name` is taken in the relation namespace, catalog or in flight.
    fn relation_taken(&self, name: &str) -> bool {
        self.schema.relation_name_taken(name) || self.claimed_relations.contains(name)
    }

    /// Whether `name` is taken in this table's constraint namespace, catalog or in
    /// flight.
    fn constraint_taken(&self, name: &str) -> bool {
        self.schema.constraint_name_taken(self.table, name)
            || self.claimed_constraints.contains(name)
    }

    /// The name a `CHECK` / `FOREIGN KEY` takes, then claimed — Postgres's
    /// `ATExecAddConstraint` split, which is the whole point of this method existing.
    ///
    /// A **declared** name is taken verbatim and a collision is the user's error
    /// (`ConstraintNameIsUsed`, per-relation → `42710`). An **auto** name is instead
    /// *chosen*, as `ChooseConstraintName` chooses it: `<table>_<name2>_<label>` cut to
    /// 63 bytes by [`make_object_name`] (an empty `name2` is `<table>_<label>`), with a
    /// digit on the **label** (`t_a_fkey`, `t_a_fkey1`, …) and the name cut again around
    /// it until nothing holds the candidate — so Postgres never lets one of its own
    /// generated names fail a statement, and two names that meet only after the cut are
    /// told apart as two that were equal whole are. Gating the auto side rejects the valid
    /// `ALTER TABLE t ADD FOREIGN KEY (a) REFERENCES p(id)` issued twice; not minting at
    /// all lands two constraints under one `(table, name)` — a pair `pg_constraint`
    /// cannot hold and an introspecting client cannot detect.
    ///
    /// The two sides search **different scopes**, deliberately (`pg_constraint.c`:
    /// "deliberately not the same test"): the declared side is this table's constraint
    /// namespace, the auto side is the whole schema's
    /// ([`Schema::constraint_name_exists`]) plus what this statement has claimed.
    ///
    /// # Errors
    ///
    /// Only for a declared name — `CatalogError::ConstraintAlreadyExists` (`42710`).
    fn mint_constraint(
        &mut self,
        declared: Option<&str>,
        name2: &str,
        label: &str,
    ) -> Result<String> {
        if let Some(name) = declared {
            self.claim_constraint(name)?;
            return Ok(name.to_string());
        }
        let candidate = choose_name(self.table, name2, label, |candidate| {
            self.schema.constraint_name_exists(candidate)
                || self.claimed_constraints.contains(candidate)
        });
        self.claimed_constraints.insert(candidate.clone());
        Ok(candidate)
    }

    /// The name a `CHECK` takes: the declared one, else `<table>_<column>_check` where
    /// the predicate reads exactly one distinct column and `<table>_check` otherwise.
    ///
    /// # Errors
    ///
    /// See [`Self::mint_constraint`].
    fn mint_check(&mut self, check: &CheckConstraintDef) -> Result<String> {
        let column = single_referenced_column(&check.expr);
        self.mint_constraint(
            check.name.as_deref(),
            column.as_deref().unwrap_or_default(),
            "check",
        )
    }

    /// The name a `FOREIGN KEY` takes: the declared one, else
    /// `<table>_<referencing columns joined by _>_fkey`
    /// (`ChooseForeignKeyConstraintNameAddition`).
    ///
    /// # Errors
    ///
    /// See [`Self::mint_constraint`].
    fn mint_foreign_key(&mut self, fk: &ForeignKeyDef) -> Result<String> {
        self.mint_constraint(fk.name.as_deref(), &fk.columns.join("_"), "fkey")
    }

    /// Claim a `CHECK` or `FOREIGN KEY` name — the constraint namespace only.
    ///
    /// # Errors
    ///
    /// `CatalogError::ConstraintAlreadyExists` (`42710`).
    fn claim_constraint(&mut self, name: &str) -> Result<()> {
        if self.constraint_taken(name) {
            return Err(crate::CatalogError::ConstraintAlreadyExists {
                constraint: name.to_string(),
                table: self.table.to_string(),
            }
            .into());
        }
        self.claimed_constraints.insert(name.to_string());
        Ok(())
    }

    /// Claim a `PRIMARY KEY` / `UNIQUE` constraint's name — both namespaces, in
    /// `index_create`'s order.
    ///
    /// # Errors
    ///
    /// `CatalogError::IndexAlreadyExists` (`42P07`) when the relation namespace holds
    /// it, else `CatalogError::ConstraintAlreadyExists` (`42710`).
    fn claim_constraint_index(&mut self, name: &str) -> Result<()> {
        if self.relation_taken(name) {
            return Err(crate::CatalogError::IndexAlreadyExists {
                index: name.to_string(),
            }
            .into());
        }
        if self.constraint_taken(name) {
            return Err(crate::CatalogError::ConstraintAlreadyExists {
                constraint: name.to_string(),
                table: self.table.to_string(),
            }
            .into());
        }
        self.claimed_relations.insert(name.to_string());
        self.claimed_constraints.insert(name.to_string());
        Ok(())
    }

    /// The index name `uc` mints, then claimed.
    ///
    /// A declared `CONSTRAINT <n>` is taken verbatim and a collision is the user's
    /// error. An **auto**-generated name is instead *chosen*, as Postgres's
    /// `ChooseIndexName` chooses a constraint's index: `<table>_pkey` for a primary key
    /// whatever its columns, `<table>_<columns joined by _>_key` for a `UNIQUE`, cut to
    /// 63 bytes by [`make_object_name`], with a digit on the label (`t_a_key`,
    /// `t_a_key1`, `t_a_key2`, …) and the name cut again around it until nothing holds
    /// the candidate. That distinction
    /// is the whole point — Postgres never lets one of its own generated names fail a
    /// statement, so gating an auto-name would reject `CREATE INDEX t_a_key ON t (b);
    /// ALTER TABLE t ADD UNIQUE (a);`, which is valid input.
    ///
    /// The candidate must clear both namespaces plus every constraint name in the
    /// schema (`isconstraint = true` in `ChooseRelationName`, which finishes with
    /// `ConstraintNameExists` — stricter than `pg_constraint`'s own per-relation index;
    /// see [`Schema::constraint_name_exists`]).
    ///
    /// # Errors
    ///
    /// Only for a declared name — see [`Self::claim_constraint_index`].
    fn mint_constraint_index(&mut self, uc: &UniqueConstraint) -> Result<String> {
        if let Some(declared) = &uc.name {
            self.claim_constraint_index(declared)?;
            return Ok(declared.clone());
        }
        let (name2, label) = if uc.is_primary {
            (String::new(), "pkey")
        } else {
            (uc.columns.join("_"), "key")
        };
        let candidate = choose_name(self.table, &name2, label, |candidate| {
            self.relation_taken(candidate)
                || self.constraint_taken(candidate)
                || self.schema.constraint_name_exists(candidate)
        });
        self.claimed_relations.insert(candidate.clone());
        self.claimed_constraints.insert(candidate.clone());
        Ok(candidate)
    }
}

/// Drop the redundant `PRIMARY KEY` / `UNIQUE` specifications of one statement,
/// reproducing Postgres's `transformIndexConstraints` (`parse_utilcmd.c`).
///
/// Two constraints over the **same key columns in the same order** describe one
/// index, and Postgres merges them rather than erroring or building two ("A strict
/// reading of SQL would suggest raising an error instead, but that strikes me as too
/// anal-retentive. - tgl"). So `CREATE TABLE t (a INT UNIQUE, UNIQUE (a))` and
/// `CREATE TABLE t (a INT PRIMARY KEY UNIQUE)` each yield exactly one index, and
/// `information_schema` reports one constraint. Column *order* is significant:
/// `UNIQUE (a, b)` and `UNIQUE (b, a)` are two indexes on both engines.
///
/// Two details are load-bearing:
/// - **the primary key is kept in preference**, so a PK merged with a UNIQUE stays a
///   PK (`pu_pkey`, `contype = 'p'`), never the other way round;
/// - **a name is transferred onto an unnamed keeper**, so `CREATE TABLE t (a INT
///   PRIMARY KEY, CONSTRAINT myu UNIQUE (a))` produces one primary key named `myu`
///   rather than `t_pkey`.
///
/// Scoped to the constraints of a *single* statement, which is also Postgres's scope:
/// `ALTER TABLE … ADD UNIQUE (a)` twice builds two indexes (`t_a_key`, `t_a_key1`) —
/// "XXX in ALTER TABLE case, it'd be nice to look for duplicate pre-existing indexes,
/// too."
fn dedup_unique_constraints(constraints: &[UniqueConstraint]) -> Vec<UniqueConstraint> {
    let pkey = constraints.iter().position(|uc| uc.is_primary);
    let mut kept: Vec<UniqueConstraint> = Vec::with_capacity(constraints.len());
    if let Some(i) = pkey {
        kept.push(constraints[i].clone());
    }
    for (i, uc) in constraints.iter().enumerate() {
        if Some(i) == pkey {
            continue;
        }
        if let Some(prior) = kept.iter_mut().find(|prior| prior.columns == uc.columns) {
            if prior.name.is_none() {
                prior.name.clone_from(&uc.name);
            }
        } else {
            kept.push(uc.clone());
        }
    }
    kept
}

/// Reject a second `PRIMARY KEY` on one table (`42P16`).
///
/// Postgres allows exactly one, and reports the multiplicity itself rather than a
/// name collision — the two may be validly named, and `CONSTRAINT aaa PRIMARY KEY`
/// against an existing `t_pkey` collides with nothing. It is checked at **every** mint
/// site (`CREATE TABLE`, `ADD CONSTRAINT`, `ADD COLUMN`), because each reaches the
/// index build by a different route; `index_check_primary_key` is likewise the one
/// place Postgres checks and every route passes through it.
///
/// Left unchecked the second key is not merely cosmetic: it lands as a second
/// constraint-marked index, `information_schema` reports two PRIMARY KEY rows, and
/// [`Schema::primary_key_index`] — a `find` over a name-sorted map — starts answering
/// by index-name order, so `REFERENCES t` binds to whichever key sorts first.
///
/// # Errors
///
/// `CatalogError::MultiplePrimaryKeys` (`42P16`).
fn ensure_no_primary_key(schema: &Schema, table: &str) -> Result<()> {
    if schema.primary_key_index(table).is_some() {
        return Err(crate::CatalogError::MultiplePrimaryKeys {
            table: table.to_string(),
        }
        .into());
    }
    Ok(())
}

/// Add the enforced unique index behind a declared `PRIMARY KEY` / `UNIQUE`
/// constraint to the catalog and build it in storage.
///
/// The **only** way a constraint-owned index is minted — every caller is a declared
/// constraint (inline, table-level, or `ALTER TABLE … ADD CONSTRAINT`), so `kind` is
/// required rather than optional and a standalone `CREATE UNIQUE INDEX` cannot reach
/// here. Calls `storage.create_index` to back-fill + maintain the entries over the
/// resolved key-column positions, and only then mints the OID and persists an `Index`
/// (with all key `columns`) into `schema.indexes` (so it re-registers on reopen) — the
/// order `execute_create_index` takes and for the same reason (`docs/ddl.md` § A failed
/// index build), and the more pressing here, since a residue on this route is
/// constraint-marked and `DROP INDEX` refuses it `2BP01`.
///
/// `index_name` is the **constraint's** name, as minted and gated by
/// [`ConstraintNames::mint_constraint_index`] — the declared `CONSTRAINT <n>` when the
/// user gave one, else a disambiguated `<table>_pkey` / `<table>_<col…>_key`
/// auto-name. Postgres names a constraint's index after the constraint, so the two are
/// one string; see [`ConstraintKind`].
///
/// The relation-name check here is a **backstop**, not the gate: a caller that minted
/// through `ConstraintNames` can never trip it, and one that forgot gets a loud
/// `42P07` instead of silently replacing the catalog entry (which strands the old
/// index's storage and stops enforcing it).
///
/// # Errors
///
/// `CatalogError::ColumnNotFound` if any `column` isn't in `table`,
/// `CatalogError::IndexAlreadyExists` if the name is taken, or a storage error from
/// `create_index`.
fn create_unique_index<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    index_name: &str,
    table: &str,
    columns: &[String],
    kind: ConstraintKind,
) -> Result<()> {
    if schema.relation_name_taken(index_name) {
        return Err(crate::CatalogError::IndexAlreadyExists {
            index: index_name.to_string(),
        }
        .into());
    }
    let table_obj = schema
        .get_table(table)
        .ok_or_else(|| crate::CatalogError::TableNotFound {
            table: table.to_string(),
        })?;
    // Validate every key column exists (its position is resolved by
    // `index_spec_from_catalog` below — this is the loud-error guard).
    validate_columns_exist(table_obj, columns)?;

    // The number the counter would hand out, read without advancing it — as
    // `execute_create_index` and `execute_create_type` read it, and for the same
    // reason: `IndexSpec` is keyed by the OID, so the fallible storage build cannot be
    // reached without a number, and a build that then fails must leave the counter
    // where it was.
    let oid = next_object_oid(schema);
    let index = Index {
        name: index_name.to_string(),
        table_name: table.to_string(),
        // Auto-created PK/UNIQUE indexes are always plain-column, full, and default
        // NULLS DISTINCT (a constraint can't be expression-keyed).
        key: IndexKey::Columns(columns.to_vec()),
        uniqueness: Uniqueness::for_constraint(kind),
        predicate: None,
        nulls_not_distinct: false,
        oid,
    };
    // Resolve the spec against the (still-borrowed) catalog table before inserting.
    let table_obj = schema
        .get_table(table)
        .ok_or_else(|| crate::CatalogError::TableNotFound {
            table: table.to_string(),
        })?;
    let spec = index_spec(&index, table_obj, &schema.enums)?;
    // The storage build is the last fallible step and runs before both catalog
    // writes, for the reason `execute_create_index` states in full: the autocommit
    // path rolls storage back and leaves `schema` mutated, so a catalog write ahead of
    // a failed build is durable — and on this route the index is **constraint-marked**,
    // so `DROP INDEX` refuses it `2BP01` and pglet has no `ALTER TABLE … DROP
    // CONSTRAINT` to reach it with. Sound only because `Storage::create_index` is
    // all-or-nothing.
    storage.create_index(spec)?;
    // The counter moves on this route's own success path rather than at the end of the
    // calling statement: two of the four call sites (`execute_create_table`'s constraint
    // pass and the `ADD COLUMN` one) are loops, and a deferred write would leave the
    // counter *behind* the OIDs the loop handed out. What that
    // costs is **monotonicity across a `DROP`**, not intra-statement distinctness —
    // `next_object_oid` re-seeds from the live catalog max, so the next iteration reads
    // past the index `schema.create_index` just registered either way; it is a dropped
    // index's number, which no longer sits in that max, that would be handed out twice
    // (`generate_object_oid`'s doc says why a reused index OID orphans a storage band).
    schema.next_oid = oid + 1;
    schema.create_index(index);
    Ok(())
}

/// Execute `DROP TABLE [IF EXISTS] name [, …] [CASCADE | RESTRICT]`.
///
/// Two classes of dependent object Postgres will not silently discard, both resolved
/// by `executor::dependencies`:
///
/// - A `FOREIGN KEY` declared on **another** table and referencing one being dropped:
///   a bare drop (or `RESTRICT`) raises `2BP01`, and `CASCADE` drops **only that
///   constraint**, leaving the child table and all its rows in place — thereafter
///   unconstrained, exactly as in Postgres. A table's own FKs (including a
///   self-reference) die with it and never block.
/// - A column `DEFAULT nextval('s')` on another table, where `s` is a sequence a
///   target **owns** and the drop would therefore take with it: `2BP01`, and `CASCADE`
///   sheds that column's default and leaves the column, its rows and the rest of its
///   table.
///
/// Without those checks each drop leaves a catalog entry pointing at something gone.
/// The FK case made every subsequent write to the child fail `23503` naming a
/// vanished table (`enforce_foreign_keys` treats a missing parent tuple as a
/// violation). The sequence case is the reason the AUTO drop and the refusal are one
/// change and not two: today's *leak* is precisely what keeps the second table's
/// default working, so dropping the sequence without the refusal would convert a leak
/// into a broken default in a table nobody named.
///
/// # Three phases, and the split is the semantics
///
/// `RemoveRelations` (`tablecmds.c`): *"First we identify all the relations, then we
/// delete them in a single `performMultipleDeletions()` call. This is to avoid
/// unwanted DROP RESTRICT errors if one of the relations depends on another."*
///
/// 1. **Resolve every name, strictly positionally** ([`resolve_relation_for_drop`]).
///    Every phase-1 error outranks every phase-2 one, and the order within phase 1 is
///    the written order — measured on 18.6 with `bp` FK-blocked, `wq` a sequence:
///    `DROP TABLE bp, nosuch_z` and its reverse are both `42P01`, `DROP TABLE bp, wq`
///    and its reverse are both `42809`, and `DROP TABLE bp, nosuch_q` is `42P01` where
///    `DROP TABLE bp` alone is `2BP01`. **`IF EXISTS` inverts the positional rule**
///    rather than weakening it: `DROP TABLE nosuch_a, wq` is `42P01` while
///    `DROP TABLE IF EXISTS nosuch_a, wq` is `42809`, because the skip removes the
///    first name's claim and lets the later wrong kind win.
/// 2. **Resolve the dependents of every survivor**, dropping the blocking entries
///    whose owning relation is itself a target ([`Dependents::retain_dependents_outside`]),
///    then `ensure_droppable`. This is what makes a mutual FK cycle droppable in one
///    statement where no single-target order works.
/// 3. **Mutate**, once nothing fallible remains — which includes sequencing the
///    fallible `storage.drop_index` calls ahead of the catalog removals they pair
///    with, since storage rolls back on the autocommit path and the catalog does
///    not. The FK removal is one merged pass
///    over the whole set ([`Dependents::drop_all_from_catalog`]) rather than one call
///    per target, because the positions are all read from the same pre-mutation
///    catalog.
///
/// Phase 3 has to be last for the reason `execute_create_table`'s doc gives: the
/// autocommit path rolls back **storage** and leaves `schema` mutated, so a statement
/// that removes one table and then raises leaves that table durably gone.
///
/// **Two divergences, both message text and neither pinnable by the fixture DSL**
/// (`@expect-error` compares SQLSTATEs). 18.6 lists *every* blocking dependent while
/// `ensure_droppable` names the first — pre-existing, and a multi-target list widens
/// the gap without changing its kind. And once two or more names survive phase 1 (a
/// duplicate counted), 18.6 switches to the plural `cannot drop desired object(s) …
/// depend on them` where pglet keeps naming one target.
pub fn execute_drop_table<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    names: &[String],
    if_exists: bool,
    cascade: bool,
) -> Result<QueryResult> {
    // Phase 1. The *survivors* are deduplicated and the *notices* are not — see
    // `resolve_drop_targets`, where the two rules and their measurements live.
    let (targets, notices) = resolve_drop_targets(schema, names, RelationKind::Table, if_exists)?;

    // Phase 2, over the survivors.
    let set: HashSet<&str> = targets.iter().copied().collect();
    let mut resolved: Vec<Dependents> = Vec::with_capacity(targets.len());
    for name in &targets {
        let target = DropTarget::Table { table: name };
        let mut dependents = Dependents::resolve(schema, target);
        dependents.retain_dependents_outside(&set);
        dependents.ensure_droppable(target, cascade)?;
        resolved.push(dependents);
    }

    // Phase 3. Drop indexes on these tables (CASCADE behavior): from the catalog AND
    // from storage — their entries AND the registered def — mirroring
    // `execute_drop_schema`, so the storage wrapper's index registry doesn't retain a
    // dropped table's defs. Keeping them is a slow leak on a long-lived server (every
    // CREATE/DROP cycle grows the registry) and a latent stale-def footgun: a backend
    // that ever reused a table id would then maintain a sibling table's index against
    // the new table's rows. `drop_indexes_for_table` returns the removed oids so the
    // storage shed can't drift from the catalog removal; `storage.drop_index` is a
    // no-op when the backend doesn't track the id, matching the `storage.drop_table`
    // tolerance below.
    //
    // **Every fallible storage call runs before the first catalog edit**, which is the
    // same order `execute_drop_index` uses and for the same reason: storage rolls back on
    // the autocommit path and the catalog does not, so a `?` *after* a catalog mutation
    // leaves an earlier target's indexes durably gone from `schema` while their storage
    // entries return — silently un-enforcing a key on a table that still exists. The
    // single-target shape had the fallible call second; over a list that window spans
    // every target, so the two are collected in separate passes rather than interleaved.
    let index_oids: Vec<u32> = targets
        .iter()
        .flat_map(|name| schema.index_oids_for_table(name))
        .collect();
    for oid in &index_oids {
        storage.drop_index(*oid)?;
    }
    for name in &targets {
        schema.drop_indexes_for_table(name);
    }
    // Shed the dependent FK constraints (under CASCADE) in **one** pass over the whole
    // set, so no target's positions are computed against a catalog another target has
    // already shortened.
    Dependents::drop_all_from_catalog(schema, &resolved, cascade);
    for name in &targets {
        schema.drop_table(name);
        // Also drop from storage (ignore errors if storage doesn't support it)
        durable_write_failure(storage.drop_table(name))?;
    }
    Ok(QueryResult::ddl_with_notices("DROP TABLE", notices))
}

/// Phase 1 of a multi-name `DROP`: resolve every written name positionally, returning
/// the **deduplicated** survivors in written order plus one `NOTICE` per *written*
/// name an `IF EXISTS` skipped.
///
/// The two halves of that sentence pull opposite ways and both are Postgres's, and
/// the discriminating statements are not the obvious ones. `DROP TABLE IF EXISTS nz,
/// nz` notices **twice** on 18.6 — the skip is per list element — while
/// `DROP TABLE IF EXISTS q1, nz, q1` notices **once**, because nothing is deleted
/// until phase 3, so the repeated `q1` still resolves. And `DROP TABLE dt, dt`
/// succeeds, so the deletion set is a set.
///
/// The dedup is currently **unobservable**: the merged catalog removal dedups its own
/// FK positions and every other phase-3 write is idempotent, so no guard in this repo
/// is red on removing it (measured). It is here because it makes "resolve, then
/// delete" true rather than accidentally true — a phase-3 write that stops being
/// idempotent would otherwise fail on the second occurrence of a name.
///
/// # Errors
///
/// The first name's error in **written** order — `42809` for a relation of the wrong
/// kind (`IF EXISTS` or not), else the kind's "does not exist".
fn resolve_drop_targets<'a>(
    schema: &Schema,
    names: &'a [String],
    kind: RelationKind,
    if_exists: bool,
) -> Result<(Vec<&'a str>, Vec<String>)> {
    let mut targets: Vec<&str> = Vec::with_capacity(names.len());
    let mut notices = Vec::new();
    for name in names {
        match resolve_relation_for_drop(schema, name, kind, if_exists)? {
            DropResolution::Found => {
                if !targets.contains(&name.as_str()) {
                    targets.push(name);
                }
            }
            DropResolution::Skipped => notices.push(drop_skip_notice(kind, name)),
        }
    }
    Ok((targets, notices))
}

/// Execute TRUNCATE.
///
/// Empties each named table (and, under `cascade`, every table that transitively
/// FK-references one of them). Unlike DELETE, TRUNCATE does **not** run `ON DELETE`
/// actions — it clears rows directly, then optionally returns the truncation
/// closure's **owned** sequences to their `START WITH` (`restart_identity`). Index
/// *entries* are shed per row via `Storage::delete`; index *definitions* are kept
/// (the tables remain).
pub fn execute_truncate<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    tables: &[String],
    cascade: bool,
    restart_identity: bool,
) -> Result<QueryResult> {
    // Every named relation must exist *and* be a table (Postgres: `42P01`, and `42809`
    // for a sequence, as for DROP TABLE). The walk is **positional**, matching
    // `ExecuteTruncate`'s own loop over `stmt->relations`: `TRUNCATE <a sequence>,
    // <missing>` is `42809` and the swap is `42P01` (measured on 18.6).
    for t in tables {
        // TRUNCATE's wrong-kind refusal is `truncate_check_rel`'s own, not
        // `validate_relation_kind`'s — `"x" is not a table`, the sentence
        // `DropErrorMsgWrongType` spells for a `DROP` from a *different* function, and
        // **without** that route's `Use DROP INDEX …` hint, which is why the route below
        // is `Truncate` (measured on 18.6 for an index and a sequence: message only,
        // against the `DROP` spelling's message-plus-hint).
        // `get_table_or_err`'s `cannot open relation "x"` + DETAIL is the
        // scan/DML wording and belongs to the other routes, so the kind check is spelled
        // out here rather than borrowed.
        if schema.get_table(t).is_none()
            && let Some(actual) = schema.relation_kind(t)
        {
            return Err(Error::WrongObjectType {
                relation: t.clone(),
                actual,
                expected: RelationKind::Table,
                route: WrongTypeRoute::Truncate,
            });
        }
        get_table_or_err(schema, t)?;
    }

    // The truncation closure and `RESTART IDENTITY`'s restart set, from the one
    // derivation `Plan::mutates_catalog` and `Plan::swapped_sequences` also read
    // (`Schema::truncation_targets`) — so neither can disagree with what this function
    // actually writes. Owned `String`s, so the storage loop below
    // holds no `schema` borrow.
    let (closure, restart_sequences) =
        schema.truncation_targets(tables, cascade, restart_identity)?;

    // Clear every row. `scan` + `delete` maintains indexes on every backend — the
    // same path DELETE uses to empty a table, minus the FK ON DELETE actions.
    for t in &closure {
        for (id, _) in storage.scan(t)? {
            storage.delete(t, id)?;
        }
    }

    // Only the counter moves: `start` and `owner` are the sequence's identity, not its
    // position, and an `insert` of a fresh record here would silently erase both.
    //
    // The target is `start`, not zero — Postgres's `ResetSequence` writes
    // `last_value = seqform->seqstart, is_called = false`, which pglet folds to
    // `start - 1`. Zeroing agrees with Postgres only for the default `START WITH 1`,
    // which is why the original fixture could not see it.
    for seq in restart_sequences {
        if let Some(sequence) = schema.sequences.get_mut(&seq) {
            sequence.last_value = sequence.start.saturating_sub(1);
        }
    }

    Ok(QueryResult::ddl("TRUNCATE TABLE"))
}

/// Execute CREATE TYPE ... AS ENUM (...).
///
/// Stores the enum (lowercased name → ordered member labels) in the catalog.
/// Enum-typed columns then resolve to `TypeOid::Text` plus the enum name (see
/// `resolve_column_type`); values are validated against the members at write time.
///
/// **The extra fold is a deliberate exception** to the rule that a name is folded
/// once, in the parser, and compared exactly thereafter — and it is
/// `to_ascii_lowercase`, **the same width as the parser's**
/// (`lexer::Token::name`), which is the load-bearing half. `name` already arrives
/// folded when the source wrote it unquoted, so at this width the fold reaches only
/// a *quoted* name: `CREATE TYPE "Loud"` registers `loud`, and
/// `CREATE TABLE t (m loud)` is then accepted here where 18.6 answers
/// `42704 type "loud" does not exist`. It is kept because the reference side cannot
/// yet spell the other answer — `parser::type_name`'s `expect_type_word` takes a bare
/// identifier only, so an enum stored under `Loud` would be nameable by no column
/// definition and, since `deparse`'s `bare_type_ident` refuses a name it cannot emit
/// unquoted, would take the export of the whole database with it. See
/// `expect_type_word` for what the reference side needs first.
///
/// **A Unicode `to_lowercase()` here would fold wider than the parser**, and the
/// excess — the non-ASCII uppercase letters `Token::name` hands through — is
/// observable on unquoted DDL with no quoting anywhere. Measured against 18.6:
/// `CREATE TYPE CAFÉ AS ENUM ('a'); CREATE TYPE café AS ENUM ('b')` registers two
/// types there (`pg_type` holds `cafÉ` and `café`) and, under the wider fold, one
/// here — so the second statement is `42710`, a `café` column binds to the *first*
/// type, and inserting `'b'` fails with `invalid input value for enum café`, naming a
/// label the user just defined. The other direction is silent: `CREATE TYPE ZÄP AS
/// ENUM ('a'); CREATE TABLE zt (c zäp)` is `42704 type "zäp" does not exist` there and
/// was accepted under the wider fold. The same width therefore has to be used by
/// `execute_drop_type` and `resolve_column_type`, the two sites that read this key
/// back; a site folding wider than its writer loses the name, and a site folding
/// wider than the parser invents a collision.
///
/// **What the fold buys is a narrower undumpable set, not an empty one**, and the
/// difference is worth stating because the argument above reads like a closure. It
/// rescues a name that differs from a safe bare identifier only in ASCII case; a
/// folded name that `is_safe_bare_ident` still rejects is undumpable exactly as
/// before, and three such statements are ordinary DDL both engines accept. Measured,
/// each on a `--data-dir` store carrying an unrelated table and its rows:
/// `CREATE TYPE "Mood-X" AS ENUM ('x','y')` (folds to `mood-x`, a `-` no bare
/// identifier may carry), `CREATE TYPE Sequence AS ENUM ('c','d')` — unquoted, and the
/// fold is not even involved — and `CREATE TYPE MÖÖD AS ENUM ('x','y')` (stores
/// `mÖÖd`; `is_safe_bare_ident` accepts ASCII lowercase only, so neither that spelling
/// nor a wider fold's `mööd` is emittable) each make `pglet_server dump --data-dir` exit `1` having
/// emitted nothing, so the unrelated table is unexportable too. 18.6 accepts all
/// three and `pg_dump` emits `CREATE TYPE public."Mood-X"` /
/// `CREATE TYPE public.sequence` / `CREATE TYPE public."mÖÖd"`. Note that the same
/// fork — a renderer refusing what it cannot spell, versus emitting what it can and
/// moving on — was decided the other way for the sequence-owner emitter, which
/// *skips* an unresolvable `OWNED BY` rather than bricking the export of every table.
/// Tracked open in `EDGES.md`.
pub fn execute_create_type<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    name: &str,
    members: &[String],
) -> Result<QueryResult> {
    let key = name.to_ascii_lowercase();
    // The type namespace holds each table's row type as well as the enums
    // ([`Schema::type_name_taken`]), so a table's name is `42710` here (`DefineEnum`'s,
    // with no hint), while an index's or a sequence's is free. It is asked of the
    // folded key the enum will be stored under **and** of the name as written: the
    // first keeps the catalog from holding a table and a type of one name, which the
    // dump cannot restore; the second is 18.6's own collision, where a quoted `"Ab"`
    // meets a table `"Ab"` that the folded key misses.
    if schema.type_name_taken(&key) || schema.tables.contains_key(name) {
        return Err(Error::DuplicateType(name.to_string()));
    }
    // **An enum's labels are a set.** 18.6 enforces it with an ordinary unique index
    // over `pg_enum (enumtypid, enumlabel)`, so the refusal is that index's `23505`
    // and it lands *behind* the type name's own `42710` — measured, and the reason
    // this sits below the check above rather than at the top.
    //
    // What the repeat costs here is not the member order: `enum_rank` reads
    // `position()`, so a second `'b'` is never ranked and `values_compare_ordered`
    // stays a total order. It costs the catalog and the dump — `pg_enum` answers two
    // rows for one label, and `CREATE TYPE e AS ENUM ('b', 'a', 'b')` is exactly the
    // text `pglet::dump` emits and stock 18.6 refuses, so the whole export then fails
    // to load, not just that type's.
    //
    // A label is a **string literal**, not an identifier, so it is compared as
    // written and never folded: `('A', 'a')` is two members on both engines.
    for (i, label) in members.iter().enumerate() {
        if members[..i].contains(label) {
            return Err(Error::UniqueViolation {
                constraint: "pg_enum_typid_label_index".to_string(),
                key: None,
            });
        }
    }
    // The number the counter would hand out, read without advancing it: the staged map
    // needs it before the last fallible step runs, and `next_oid` is a catalog fact like
    // any other. Advanced below, once the rebuild is behind us.
    let oid = next_object_oid(schema);
    // A name a live column still declares can be *re-*created here, which moves that
    // column's member order (see `rebuild_indexes_over_enum`). The rebuild is fallible
    // and runs **before** the catalog write, against the map this statement is about to
    // install — the staging § on `rebuild_indexes_over_enum` says why the map has to be
    // a value rather than a mutation to undo.
    let mut staged = schema.enums.clone();
    staged.insert(
        key.clone(),
        EnumType {
            oid,
            labels: members.to_vec(),
        },
    );
    rebuild_indexes_over_enum_staged(schema, storage, &key, &staged)?;
    schema.next_oid = oid + 1;
    schema.enums = staged;
    Ok(QueryResult::ddl("CREATE TYPE"))
}

/// Re-derive and rebuild every storage index whose maintenance reads the enum named
/// `key`, after a statement moved `Schema.enums[key]` under it.
///
/// # A frozen copy of a movable fact
///
/// `IndexSpec::column_enum_orders` is a `to_vec` of the member list, taken once at
/// `CREATE INDEX` and on reopen, because `pglet_storage` holds no catalog to borrow
/// from. It was modelled on `IndexSpec::column_types`, which is safe with that lifetime
/// for a reason this field does not share: a column's declared **type** cannot change
/// here (`ALTER COLUMN … SET DATA TYPE` is `0A000`), while an enum's **member list**
/// can — `DROP TYPE` then `CREATE TYPE` with the members in another order is accepted
/// (18.6 raises `2BP01` for the drop; pglet's missing dependency check is a separate,
/// pre-existing gap). Making the member order load-bearing turned that leak into a
/// **silent wrong write**: over `r3(id, k, m m6 NOT NULL)` under
/// `CREATE UNIQUE INDEX r3_u ON r3 (k) WHERE m > 'ok'`, reordering `m6` left the
/// executor's per-statement `PreparedIndex` on the live order and the storage
/// side on the frozen one, so two rows sharing `k` were both accepted where 18.6
/// refuses the reorder outright.
///
/// # Why the entries move too
///
/// Re-registering the definition alone would fix maintenance from here on and leave a
/// silent wrong write behind it: a row stored *before* the move was placed in — or
/// kept out of — the index by the old predicate verdict, so a later duplicate probes an
/// index the live rule says already holds its twin and finds nothing. `rebuild_index`
/// re-derives the definition **and** the entries from the live catalog and the current
/// rows, which is the only self-consistent state (and is why `drop_index` +
/// `create_index` is not the idiom here — see that method's own doc).
///
/// # Which indexes
///
/// Keyed on the **table's** columns rather than on the index's key or predicate:
/// `column_enum_orders` is table-wide and positionally parallel to every column, so
/// "this table has a column of this enum" is exactly the condition under which the
/// spec's bytes change. A narrower test — does the predicate mention the column —
/// would be a second derivation of the same fact.
///
/// Every fallible derivation (`index_spec_from_catalog`) runs before the first storage
/// mutation.
///
/// # Staged, because this whole operation is fallible and runs at DDL time
///
/// A failed DDL statement must leave the catalog untouched — the autocommit path cannot
/// put it back — so the two `CREATE TYPE` / `DROP TYPE` callers hand
/// [`rebuild_indexes_over_enum_staged`] the enum map they are *about to* install rather
/// than mutating `Schema.enums` first and calling this. That ordering is not a
/// precaution: it is **measured**. Over `rf(id, d INT, m m3)` under
/// `CREATE INDEX rf_f ON rf ((10 / d)) WHERE m > 'ok'` holding one `'sad'` row whose `d`
/// is zero, the index builds empty (the predicate excludes the row) and evaluates no key;
/// reordering `m3` then admits the row, its key is `22012` — and with the write first,
/// both `DROP TYPE m3` and the
/// re-`CREATE TYPE` reported that error while the catalog kept the move, leaving a type
/// whose members no statement claims to have set.
///
/// The **restore** callers do not stage, and correctly: each runs its resync after
/// `Schema::restore_from`, because there the catalog write *is* the intended outcome,
/// rather than a step that has to be undoable. There are three of them and they are in
/// two crates — [`enums_moved_by_restore`] enumerates them.
///
/// # The storage loop is the second mutable side, and staging alone pointed the desync
/// the other way
///
/// Staging keeps the **catalog** on the old order when the rebuild fails; it says
/// nothing about the indexes the loop had already moved. Over two tables carrying the
/// same enum — a unique partial index on one, a `float8`-keyed partial index on the
/// other — the loop rebuilt the first onto the staged order, raised on the second, and
/// left the executor (reading `Schema.enums`, unmoved) and storage (holding the staged
/// order for index one) disagreeing about the same predicate. Measured on the default
/// B+tree backend: `DROP TYPE` reports the encoder's refusal and two rows sharing the
/// unique key both land, `(2 rows)` where the same script without the failing statement
/// refuses the second with `23505`. Which way it falls is `Schema.indexes`' `BTreeMap`
/// iteration — **lexical index-name order** — so renaming the `float8` index to sort
/// first refuses the duplicate correctly. Nor is the block spelling a repair:
/// `BEGIN; DROP TYPE …; ROLLBACK;` gives the same two rows, because
/// `enums_moved_by_restore` compares a catalog that staging never moved and finds
/// nothing to resync.
///
/// So the loop **unwinds**: it derives both orders up front and, on a failure, rebuilds
/// the indexes it had already moved back onto the order they came from. That the
/// *failing* index needs no unwinding is [`Storage::rebuild_index`]'s all-or-nothing
/// contract, not an assumption about which step raised. A failed unwind cannot be
/// papered over — the specs are then genuinely out of step — so it is reported in place
/// of the statement's own error, naming both.
///
/// The unwind's `from` order is `schema.enums`, which is the pre-statement map at the
/// two staging callers and the *destination* at every restore caller — where the two
/// maps are one, nothing is being moved back, and the unwind re-derives the same bytes.
///
/// At a restore caller that is the **hole** rather than a reassurance, and it is live:
/// the restore writes the catalog before calling here, so a resync that raises part-way
/// leaves the catalog on the restored order while an index it had not reached keeps the
/// one the block installed — and the unwind, reading the destination as its `from`, puts
/// nothing back. Measured on the B+tree backend at `TransactionSession::rollback`, where
/// what it leaves is a silent wrong write: `EDGES.md` row 110.
///
/// The **shape** is every caller [`enums_moved_by_restore`] names: none of them stages,
/// so none of them has anything to put back. What only two of them have is the measured
/// **reach**. Reaching it takes a row that satisfies a partial index's predicate under
/// the order being restored *to* while carrying a key that cannot be derived. The two
/// spellings that build such a pair *directly* both raise up front: over
/// `rb(id, d INT, m mood NOT NULL)` under `CREATE INDEX a_f ON rb ((10 / d)) WHERE m > 'ok'`,
/// `INSERT INTO rb VALUES (1, 0, 'happy')` and a `CREATE INDEX` over a table already
/// holding that row each raise the key's `22012`. What is left is
/// moving the *order* under an existing pair, and that is the route row 110's script
/// takes: a second connection commits the row while the block holds the moved order.
/// The `DO` block's undo at [`execute_do_block`] — a caught body's, and a failed
/// statement's — restores to the block's own entry state instead (`rollback_to_savepoint`
/// runs first, so the rows are the entry rows), and no second connection can commit
/// inside one statement: `Plan::DoBlock` is never read-only, so the native server runs it under `lock_both()` held across
/// `execute_with_storage` on either of the two branches that reach it, and the embed and
/// wasm hosts run a statement behind `&mut self`, so a second logical connection there
/// (`Database::execute_as`) cannot interleave one. No route has been constructed there; the
/// attempt that comes closest is measured at [`execute_do_block`].
///
/// # Errors
///
/// Propagates a spec-serialization failure, or a storage rebuild failure. The back-fill
/// is `create_index`'s own, so it carries that path's residual: a *unique* index whose
/// rows now all satisfy the predicate is rebuilt holding the duplicates rather than
/// refusing (`EDGES.md`).
///
/// [`execute_do_block`]: crate::executor::execute_do_block
pub fn rebuild_indexes_over_enum<S: Storage>(
    schema: &Schema,
    storage: &mut S,
    key: &str,
) -> Result<()> {
    rebuild_indexes_over_enum_staged(schema, storage, key, &schema.enums)
}

/// [`rebuild_indexes_over_enum`] against an enum map that is **not** `schema.enums` —
/// the one a statement is about to install, so its fallible half runs before its
/// catalog write.
///
/// `schema` still supplies the tables and the indexes; only the enums come from
/// `enums`. The "which indexes" test reads `table.columns`, which the enum map does not
/// affect, so staging changes *what the specs say* and never *which* are rebuilt.
///
/// # Errors
///
/// As [`rebuild_indexes_over_enum`].
pub fn rebuild_indexes_over_enum_staged<S: Storage>(
    schema: &Schema,
    storage: &mut S,
    key: &str,
    enums: &BTreeMap<String, EnumType>,
) -> Result<()> {
    // Both orders, derived before the first storage mutation: the one this call moves
    // the specs *to*, and the one it moves them **from** and must be able to put back.
    // Taking the two as a pair is total because the `None` verdict is the key-column
    // resolution's and never reads `enums` — this function's header states that, and it
    // is what would have to move for an index to be silently left out of a move.
    let mut moves = Vec::new();
    for index in schema.indexes.values() {
        let Some(table) = schema.get_table(&index.table_name) else {
            continue;
        };
        if !table
            .columns
            .iter()
            .any(|c| c.enum_type.as_deref() == Some(key))
        {
            continue;
        }
        if let (Some(from), Some(to)) = (
            index_spec_from_catalog(index, table, &schema.enums)?,
            index_spec_from_catalog(index, table, enums)?,
        ) {
            moves.push((from, to));
        }
    }
    for (i, (_, to)) in moves.iter().enumerate() {
        let Err(failure) = storage.rebuild_index(to.clone()) else {
            continue;
        };
        // The specs already moved go back to the order they came from. `rebuild_index`
        // is all-or-nothing, so the one that just failed is still on `from` and is not
        // in this range.
        for (from, _) in &moves[..i] {
            if let Err(unwound) = storage.rebuild_index(from.clone()) {
                return Err(Error::Execution(format!(
                    "index specs left out of step with the catalog after a failed enum \
                     rebuild: {unwound} (the statement failed with: {failure})"
                )));
            }
        }
        return Err(failure);
    }
    Ok(())
}

/// [`rebuild_indexes_over_enum`] over **every** enum name in `moved` — the form a
/// caller takes when it restored a whole catalog rather than executing one statement.
///
/// The rule the two share: *the storage index specs follow `Schema.enums` wherever it
/// moves.* A catalog restore moves it **back**, which is a move like any other, and
/// `rebuild_index` is idempotent and derived only from `(spec, current rows)` precisely
/// so the backward move can re-run it.
///
/// `moved` comes from [`enums_moved_by_restore`], and the pair is what every backward
/// caller runs; the callers are enumerated there.
///
/// # Errors
///
/// As [`rebuild_indexes_over_enum`].
pub fn rebuild_indexes_over_enums<S: Storage>(
    schema: &Schema,
    storage: &mut S,
    moved: &[String],
) -> Result<()> {
    for key in moved {
        rebuild_indexes_over_enum(schema, storage, key)?;
    }
    Ok(())
}

/// The enum names a catalog restore is about to **move** — those `snapshot` and the
/// live `schema` disagree on, by name or by member list.
///
/// Paired with [`rebuild_indexes_over_enums`], which every caller runs over the answer
/// *after* its `Schema::restore_from`; this half must run **before** it, since it is a
/// difference against the catalog the restore is replacing. Computed from a symmetric
/// difference, so a name added, removed, or reordered in either direction is named once.
///
/// Comparing rather than always resyncing is what keeps an ordinary `ROLLBACK` — every
/// fixture runs one — from re-deriving every index spec in the database.
///
/// **The callers are every backward `Schema::restore_from`, and the enumeration is the
/// point of this function living here.** `Schema::restore_from` has three call sites and
/// they are in two crates: `pglet::transaction`'s `TransactionSession::rollback` and
/// `::rollback_to_savepoint`, and this crate's own [`execute_do_block`] undo — run for a
/// caught body before its handler and for a `DO` statement that fails, which restores
/// the catalog exactly as those two do. That
/// third one was written without the pair and desynced silently: a `DROP TYPE` +
/// re-`CREATE TYPE` reorder inside a caught body moved `Schema.enums` and the index
/// specs forward, the handler put the catalog back, and the specs kept the block's
/// order — so a unique partial index over `WHERE m > 'ok'` took a duplicate with no
/// error on any statement. 18.6 has no comparable state to compare against (its own
/// `DROP TYPE` under a live dependent column is `2BP01`); what the script violates is
/// this engine's own invariant, that its catalog and its index specs agree.
///
/// [`execute_do_block`]: crate::executor::execute_do_block
#[must_use]
pub fn enums_moved_by_restore(schema: &Schema, snapshot: &Schema) -> Vec<String> {
    let mut moved: Vec<String> = Vec::new();
    for (name, e) in &schema.enums {
        if snapshot.enums.get(name).map(|s| &s.labels) != Some(&e.labels) {
            moved.push(name.clone());
        }
    }
    for name in snapshot.enums.keys() {
        if !schema.enums.contains_key(name) {
            moved.push(name.clone());
        }
    }
    moved
}

/// Execute DROP TYPE.
///
/// Folds at `execute_create_type`'s width — ASCII, the parser's — because this is the
/// lookup for the key that function wrote: `CREATE TYPE ZÄP` registers `zÄp`, and a
/// Unicode `to_lowercase()` here would ask for `zäp` and report the type it just
/// created as missing. The `IF EXISTS` spelling is where that is *silent* (a skip is a
/// legal outcome), which is why the fixture drives both.
pub fn execute_drop_type<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    name: &str,
    if_exists: bool,
    cascade: bool,
) -> Result<QueryResult> {
    let key = name.to_ascii_lowercase();
    // A table's row type is in the type namespace ([`Schema::type_name_taken`]), so it
    // is *found* — and refused, as 18.6 refuses it through the table's internal
    // dependency, ahead of the `IF EXISTS` skip, which only a name no type holds
    // reaches. The row type is the table's name as written, so it is read unfolded,
    // and **ahead of the enum lookup**, which reads the folded key: `DROP TYPE "Rd_Ab"`
    // beside a table `"Rd_Ab"` and an enum `rd_ab` names the row type, and dropping
    // the enum instead would drop an object the statement did not name. The enum wins
    // only where it holds the name as written byte for byte — a table and an enum of
    // one name, which no statement builds now and a store written before the row-type
    // gates can hold; dropping the enum is then how that pair is parted without
    // dropping the table — where no column declares it, since a `DROP TYPE` under a
    // live dependent column is accepted here and leaves the column declaring a type the
    // dump never creates — and until it is parted the dump refuses the store.
    let enum_holds_the_name = key == name && schema.enums.contains_key(&key);
    if !enum_holds_the_name && schema.tables.contains_key(name) {
        return Err(Error::TypeRequiredByTable(crate::lexer::quote_ident(name)));
    }
    if !schema.enums.contains_key(&key) {
        if !if_exists {
            return Err(Error::Execution(format!("type \"{name}\" does not exist")));
        }
        return Ok(QueryResult::ddl_with_notice(
            "DROP TYPE",
            format!("type \"{name}\" does not exist, skipping"),
        ));
    }
    // A persisted expression that converts to this enum names it — `CHECK (s::mq IS
    // NOT NULL)`, `DEFAULT ('a'::text)::mq` — and carries its member list
    // (`planner::StringFunc::EnumIn`); a dump renders it `(s)::mq`, which no engine loads
    // once the type is gone. 18.6 refuses the drop through the dependency edge and
    // `CASCADE` drops the dependents, and the one resolver does both here. A **column**
    // declaring the enum is not an edge it enforces: that is `EDGES.md` row 261's.
    let target = DropTarget::Type { name: &key };
    let dependents = Dependents::resolve(schema, target);
    dependents.ensure_droppable(target, cascade)?;
    // A column declaring the dropped name now orders by label bytes (`enum_order_of`
    // answers `None`), which is a move of the member order like any other — so the
    // indexes follow it here too, not only at the re-`CREATE TYPE`. Staged before the
    // removal, for the reason `rebuild_indexes_over_enum` states. Every storage step —
    // this and a cascaded index's data — runs before the first catalog write, which is
    // the failed-DDL rule (`docs/ddl.md`).
    let mut staged = schema.enums.clone();
    staged.remove(&key);
    rebuild_indexes_over_enum_staged(schema, storage, &key, &staged)?;
    if cascade {
        for dependent in &dependents.blocking_expressions {
            if let ExpressionRef::Index { name } = dependent
                && let Some(index) = schema.get_index(name)
            {
                storage.drop_index(index.index_id())?;
            }
        }
    }
    dependents.drop_from_catalog(schema, target, cascade);
    schema.enums = staged;
    Ok(QueryResult::ddl("DROP TYPE"))
}

/// Execute `DROP SCHEMA [IF EXISTS] public [CASCADE]`.
///
/// pglet models a single implicit `public` schema, so this clears the whole
/// catalog (every table, index, enum, and sequence) and its storage — the
/// migration/test reset idiom (`DROP SCHEMA public CASCADE; CREATE SCHEMA
/// public`). A non-`public` name is "not found" (`3F000`, or a no-op under
/// `IF EXISTS`); a `public` holding a table, an enum or a sequence refuses a drop
/// without `CASCADE` (`2BP01`), matching Postgres, before anything is removed. An
/// extension is not among them — pglet keeps no installation state — so a `public`
/// holding only one accepts the drop where 18.6 refuses it. Built from the same
/// storage primitives as `execute_drop_*`, so it commits on the auto-commit path and
/// rolls back inside a transaction (the server snapshots the schema on `BEGIN` and
/// discards buffered writes on `ROLLBACK`).
pub fn execute_drop_schema<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    name: &str,
    if_exists: bool,
    cascade: bool,
) -> Result<QueryResult> {
    if name != "public" {
        // Only the implicit `public` schema exists; any other name is "not found".
        // **Exact**: the parser downcased an unquoted `PUBLIC`, so a quoted `"PUBLIC"`
        // reaching here names a schema that does not exist — `3F000` on 18.6, and the
        // difference between refusing and **dropping the real `public` schema with
        // every table in it**.
        if if_exists {
            return Ok(QueryResult::ddl_with_notice(
                "DROP SCHEMA",
                format!("schema \"{name}\" does not exist, skipping"),
            ));
        }
        return Err(Error::SchemaNotFound(name.to_string()));
    }

    // Postgres rejects dropping a non-empty schema without CASCADE (2BP01). The
    // objects are what the clear below removes, less the one kind that cannot be
    // there alone: an index belongs to a table. A sequence counts on its own; left
    // out, one would be cleared, counter and all, where 18.6 refuses.
    let has_objects =
        !schema.tables.is_empty() || !schema.enums.is_empty() || !schema.sequences.is_empty();
    if has_objects && !cascade {
        return Err(Error::DependentObjectsExist("public".to_string()));
    }

    // Drop index data first (mirrors `execute_drop_index`), then table data
    // (mirrors `execute_drop_table`); take owned copies so the storage loop holds
    // no borrow of `schema`.
    let index_oids: Vec<u32> = schema.indexes.values().map(Index::index_id).collect();
    for oid in index_oids {
        storage.drop_index(oid)?;
    }
    let table_names: Vec<String> = schema.tables.keys().cloned().collect();
    for t in &table_names {
        // `drop_table` may be a no-op on backends that don't track it; ignore (the
        // catalog clear below is authoritative), matching `execute_drop_table`.
        durable_write_failure(storage.drop_table(t))?;
    }

    schema.tables.clear();
    schema.indexes.clear();
    schema.enums.clear();
    schema.sequences.clear();

    Ok(QueryResult::ddl("DROP SCHEMA"))
}

/// Execute `CREATE SCHEMA [IF NOT EXISTS] public`.
///
/// pglet models a single always-present implicit `public` schema, so creating
/// `public` is a no-op success — the second half of the reset idiom, where a
/// preceding `DROP SCHEMA public CASCADE` has emptied it. (This diverges from a
/// fresh Postgres, which would raise `42P06` for a bare `CREATE SCHEMA public`
/// since `public` already exists; pglet has no notion of the schema's presence to
/// check, and the reset idiom needs the create to succeed.) Any other schema name
/// fails loud — pglet cannot model a second namespace.
pub fn execute_create_schema(name: &str, if_not_exists: bool) -> Result<QueryResult> {
    // `public` always "exists", so `IF NOT EXISTS` changes nothing either way.
    let _ = if_not_exists;
    // **Exact**, for the same reason `execute_drop_schema` is: a quoted `"PUBLIC"`
    // names a second schema on 18.6 (which creates it, and answers `42P06` on a
    // repeat). pglet models one namespace, so it lands on the loud `0A000` below —
    // a divergence, but the documented one, and an improvement on the silent
    // success it used to report.
    if name != "public" {
        return Err(Error::NotImplemented(format!(
            "CREATE SCHEMA \"{name}\": pglet supports only the implicit \"public\" schema"
        )));
    }
    Ok(QueryResult::ddl("CREATE SCHEMA"))
}

/// Execute ALTER TABLE ADD COLUMN.
///
/// The added column's inline constraints all wire to real enforcement, matching
/// Postgres (not silently dropped):
/// - a `DEFAULT` **backfills every existing row** (Postgres fills them, not NULL) —
///   a stable default (literal / `now()` / `CURRENT_DATE`|`TIME`) is evaluated once
///   for the whole statement, a volatile one (`gen_random_uuid()` / `nextval`/SERIAL)
///   per row; a `NOT NULL` column with no `DEFAULT` on a non-empty table is `23502`;
/// - an inline `UNIQUE`/`PRIMARY KEY` builds an enforced unique index (`23505` if the
///   backfilled values collide);
/// - an inline `CHECK` is recorded + enforced (`23514` if a backfilled row violates it);
/// - an inline `REFERENCES parent(col)` FK is recorded (`23503` if a non-NULL
///   backfilled value has no live parent).
///
/// Everything fallible is computed + validated **before** the mutation block: the
/// autocommit path doesn't restore the schema on error, and the memory backend
/// doesn't roll back storage, so a would-be-violating backfill must abort untouched.
///
/// Be exact about the two halves of that, because the sentence used to be written as
/// one and was false in a way that wrote. **Before** the block, the one catalog fact
/// the backfill itself needs — the sequence a `SERIAL` / identity / user `nextval`
/// default draws from — is *staged* rather than registered, since registering it left
/// each of `23514` / `23505` / `23503` raising over a durable phantom
/// sequence carrying the failed backfill's own advances (see `staged_sequences`).
/// The `23502` gate stands between the two but cannot fire over such a column — a
/// `NextVal` default resolves to an `int8` and no coercion makes that NULL, so
/// `ADD COLUMN c SERIAL NOT NULL` succeeds on both engines.
/// **Inside** the block nothing is claimed: the column lands in the catalog while the
/// widened rows roll back with storage, which is the wire-brick class recorded in
/// `EDGES.md` — a separate, wider repair.
///
/// One route into that class is closed here and it is **one route, not the class**:
/// the inline `UNIQUE` / `PRIMARY KEY` index build used to be a fallible step *after*
/// the mutation block opened, so a key the build refused bricked the table. Each
/// refusal of that build is now asked ahead of the block, pglet's own — the page bound
/// over the back-filled keys — last. Everything else inside the block —
/// `storage.update` / `add_column` failing mid-back-fill — is untouched. The same `create_unique_index` reached from
/// `execute_create_table` or `ALTER TABLE … ADD CONSTRAINT` is closed the other way:
/// there the index's own catalog writes sit **below** `storage.create_index` rather
/// than behind a pre-flight (`docs/ddl.md` § A failed index build).
#[allow(clippy::too_many_arguments)]
pub fn execute_alter_table_add_column<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    table_name: &str,
    column: &crate::planner::ColumnDef,
    foreign_key: Option<&ForeignKeyDef>,
    column_uniques: &[UniqueConstraint],
    checks: &[CheckConstraintDef],
    if_not_exists: bool,
) -> Result<QueryResult> {
    // Column-existence check (read-only) — an early return before any mutation, and
    // **ahead of the declared type**, which is where 18.6 puts it and where the two
    // DDL statements genuinely part company. `ALTER TABLE`'s own
    // relation lookup raises the `42P01` before any command runs, and then
    // `ATExecAddColumn` calls `check_for_column_name_collision` *before* the
    // `BuildDescForRelation` that resolves the column's type — the comment between
    // the two in `tablecmds.c` says so outright ("we mustn't do it until we have made
    // the if_not_exists check"). So `42P01`, `42701` and the `IF NOT EXISTS` skip each
    // outrank `42704`, `42601 type modifier is not allowed` and the identity `22023`
    // alike, measured on 18.6. `CREATE TABLE` is the other way round (see
    // `execute_create_table`: the `tableElts` walk precedes even the `42P07`),
    // so pglet mirrors the asymmetry rather than normalising it.
    //
    // `check_for_column_name_collision` answers **two** names, and the system-column
    // half is not the user-column half wearing a different message: it is the arm
    // 18.6 reaches when the name resolves with `attnum <= 0`, and it raises with
    // no `if_not_exists` test in the path. So `ALTER TABLE t ADD COLUMN IF NOT EXISTS
    // ctid int` is `42701` and not a skip, measured — the skip belongs to a
    // collision with a column the *user* declared, which is the only kind an
    // `ADD COLUMN` could sensibly be re-running.
    {
        if is_system_column_name(&column.name) {
            // Read before the relation's own columns, matching the arm order of the
            // one 18.6 lookup, which branches on `attnum <= 0` before it can
            // reach the *already exists* arm. The gate above closes the producers and
            // not the state, so a legacy store *can* hold a user column of this name;
            // this order is what decides which sentence that table then gets.
            get_table_or_err(schema, table_name)?;
            return Err(crate::CatalogError::SystemColumnName {
                column: column.name.clone(),
            }
            .into());
        }
        let table = get_table_or_err(schema, table_name)?;
        if table.columns.iter().any(|c| c.name == column.name) {
            if if_not_exists {
                return Ok(QueryResult::ddl("ALTER TABLE"));
            }
            return Err(crate::CatalogError::ColumnAlreadyExists {
                table: table_name.to_string(),
                column: column.name.clone(),
            }
            .into());
        }
    }

    // Resolve the type (incl. enum lookup) before the mutable table borrow. Fails
    // loud on an unsupported/unrecognized type (unless `coerce_unknown_types`) and on
    // a `(…)` modifier the type carries none of.
    let rt = resolve_column_type(&column.type_name, schema)?;
    // An `ADD COLUMN … GENERATED … AS IDENTITY` column must be integer-typed (`22023`)
    // — **after** the type's own verdict, which is 18.6's order: over an existing `ex`,
    // `… ADD COLUMN z nosuchtype GENERATED ALWAYS AS IDENTITY` is `42704` there and
    // `… z int4(3) GENERATED …` is `42601`, not the `22023` a `text` identity gets.
    if column.identity.is_some() {
        ensure_identity_integer(rt.type_oid)?;
    }

    // The default is cooked here, right behind the gates it loses to on this route —
    // the relation, then the column name, then the declared type, in that order — and
    // ahead of every gate it beats. The column-name gate is one gate answering **two**
    // names, as `check_for_column_name_collision` does, so both spellings are above the
    // cook. Measured on 18.6: `ALTER TABLE
    // nosuchtbl ADD COLUMN b INT DEFAULT 'x'` is `42P01`, `… b NOSUCHTYPE DEFAULT 'x'`
    // is `42704`, `… ADD COLUMN a INT DEFAULT 'x'` over an existing `a` is
    // `42701` and `… ADD COLUMN ctid INT DEFAULT 'x'` is `42701` too, while the
    // inline `PRIMARY KEY` multiplicity, a declared
    // constraint-index name already taken, an ill-typed inline `CHECK` and a missing
    // inline `REFERENCES` parent are each `22P02` — the default's own error. An
    // `implicit_sequence` column carries no user default (`SERIAL DEFAULT 5` is
    // `42601` at parse, as on 18.6).
    if let Some(default) = &column.default
        && !column.implicit_sequence
    {
        let probe_column = Column {
            name: column.name.clone(),
            type_oid: rt.type_oid,
            nullable: column.nullable,
            position: 0,
            default: None,
            enum_type: rt.enum_type.clone(),
            numeric_typmod: rt.numeric_typmod,
            varchar_length: rt.varchar_length,
            temporal_precision: rt.temporal_precision,
            identity: column.identity,
        };
        cook_column_default(default, &probe_column, DefaultTiming::Eager, schema)?;
    }

    // An inline `PRIMARY KEY` on the added column reaches the same index build as
    // every other route, so it gets the same multiplicity gate — and at the same point
    // in the order Postgres puts it, measured on 18.6: after the type (`42704`) and the
    // column-already-exists check (`42701`), before the inline `REFERENCES` parent
    // (`42P01`) and before the backfill's own `23502`/`23505`.
    let column_uniques = &dedup_unique_constraints(column_uniques)[..];
    if column_uniques.iter().any(|uc| uc.is_primary) {
        ensure_no_primary_key(schema, table_name)?;
    }

    // A SERIAL / identity column's implicit sequence is named here, against the live
    // relation namespace, and its name is seeded into `ConstraintNames` below —
    // `transformAlterTableStmt` schedules the `CreateSeqStmt` into the *before*-list,
    // so the sequence exists by the time an inline `UNIQUE`'s index is named.
    // Measured on 18.6: with a sequence already named `d1_y_key`,
    // `ALTER TABLE d1 ADD COLUMN y SERIAL UNIQUE` takes `d1_y_seq` for the sequence
    // and pushes the index to `d1_y_key1`. A **declared** `SEQUENCE NAME` is taken
    // verbatim here and refused (`42P07`) if the namespace holds it.
    let implicit_sequence = column
        .implicit_sequence
        .then(|| {
            resolve_implicit_sequence_name(
                schema,
                &HashSet::new(),
                table_name,
                &column.name,
                column.sequence_name.as_deref(),
            )
        })
        .transpose()?;

    // The sequence a user's own `DEFAULT nextval('x')` will register further down. Like
    // the implicit one it is **claimed** into `ConstraintNames` below, because Postgres's
    // relation namespace already holds a sequence a `DEFAULT nextval('x')` it accepts
    // names — measured on 18.6 with the sequence pre-created, `ALTER TABLE d2 ADD COLUMN
    // y INT DEFAULT nextval('d2_y_key') UNIQUE` takes `d2_y_key1` for the index. Without
    // the claim the auto-name landed on the sequence and only `create_unique_index`'s
    // post-mutation backstop caught it, which on this route is the worst shape in the
    // engine: the column is in the catalog, the rows were rolled back one value short,
    // and every `SELECT *` on that table then failed at the wire, for every client,
    // permanently. Its kind gate runs further down, after the naming (see below).
    // Read through `default_sequence_refs` (see `execute_create_table`'s twin), so an
    // expression default's sequences are claimed and kind-gated exactly as the bare
    // spelling's is — and **all** of them, since 18.6 records a `pg_attrdef`
    // dependency per sequence function in the tree.
    let user_default_sequences: Vec<String> = column
        .default
        .as_ref()
        .map(default_sequence_refs)
        .unwrap_or_default();

    // The catalog `Column` for the added column, built up front so the DEFAULT
    // backfill resolves through the same `resolve_default` (typmod + coercion) path
    // every other write uses.
    let new_column = {
        let table = get_table_or_err(schema, table_name)?;
        Column {
            name: column.name.clone(),
            type_oid: rt.type_oid,
            nullable: column.nullable,
            position: (table.columns.len() + 1) as i16,
            default: implicit_sequence.clone().map_or_else(
                || column.default.clone(),
                |s| Some(ColumnDefault::NextVal(s)),
            ),
            enum_type: rt.enum_type,
            numeric_typmod: rt.numeric_typmod,
            varchar_length: rt.varchar_length,
            temporal_precision: rt.temporal_precision,
            identity: column.identity,
        }
    };

    // Name every inline constraint through the one gate — **index first, then the
    // CHECKs, then the FK**, which is *not* `CREATE TABLE`'s order and is not derivable
    // from it. The mechanism is a reschedule, not the pass numbering: `ALTER TABLE`'s
    // passes run ascending and `AT_PASS_ADD_CONSTR` (6) is *ahead* of `AT_PASS_ADD_INDEX`
    // (9), but nothing final happens in pass 6. `transformAlterTableStmt` turns the
    // inline `UNIQUE` into an `AT_AddIndex` subcommand scheduled straight into pass 9,
    // while the CHECK and the FK become `AT_AddConstraint`s scheduled into pass 6 — and
    // there `ATExecCmd` calls `ATParseTransformCmd`, whose only job that pass is to
    // *re-file* them into `AT_PASS_ADD_OTHERCONSTR` (10). So the index lands in 9 and
    // both of the others in 10, the CHECKs ahead of the FK because
    // `transformAlterTableStmt` appends `ckconstraints` before `fkconstraints`.
    // `DefineRelation` instead attaches the CHECKs as the heap is created and builds the
    // indexes after. Both are observable and they point opposite ways (measured on
    // 18.6): `CREATE TABLE ll (c INT CONSTRAINT ll_c_check UNIQUE CHECK (c > 0))` is
    // `42710` — the CHECK auto-names `ll_c_check` first and the declared UNIQUE collides
    // — while `ALTER TABLE ii ADD COLUMN c INT CONSTRAINT ii_c_check UNIQUE CHECK (c >
    // 0)` **succeeds** as `ii_c_check` + `ii_c_check1`, the CHECK yielding to the index.
    // The index also wins over the CHECK's *type* check there (`CONSTRAINT taken UNIQUE
    // CHECK (int_col LIKE 'x')` against an existing `taken` is `42710`, not `42883`), so
    // the whole index step precedes the CHECK loop rather than only its naming.
    //
    // All read-only, and all **before** the inline `REFERENCES` parent is resolved,
    // which is where Postgres puts them (an ill-typed inline CHECK is `42883` and a
    // duplicate inline CHECK name is `42710`, both ahead of the FK's `42P01`; a
    // *declared* FK name collision likewise beats its own missing parent). Also before
    // the backfill validation, because a name collision outranks the `23505` the index
    // build would raise.
    let (named_checks, index_names, fk_name) = {
        let mut names = ConstraintNames::new(schema, table_name);
        names.seed_relations(
            implicit_sequence
                .iter()
                .chain(user_default_sequences.iter())
                .cloned()
                .collect(),
        );
        let index_names = column_uniques
            .iter()
            .map(|uc| names.mint_constraint_index(uc))
            .collect::<Result<Vec<_>>>()?;
        let table = get_table_or_err(schema, table_name)?;
        // The post-ADD column set (existing columns plus the new one) — the scope the
        // inline CHECKs resolve against.
        let mut cols = table.columns.clone();
        cols.push(new_column.clone());
        let named_checks: Vec<CheckConstraint> = checks
            .iter()
            .map(|check| {
                validate_check_exprs(std::slice::from_ref(check), &cols, &schema.enums)?;
                let name = names.mint_check(check)?;
                Ok(CheckConstraint {
                    name,
                    expr: check.expr.clone(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let fk_name = foreign_key
            .map(|fk| names.mint_foreign_key(fk))
            .transpose()?;
        (named_checks, index_names, fk_name)
    };

    // Validate + resolve an inline `REFERENCES` FK on the added column **read-only,
    // before any mutation**, so a bad FK (missing parent `42P01` / no covering unique
    // key `42830` / arity) aborts the ADD COLUMN with the catalog untouched. The
    // referenced key lives on the parent — already in the catalog, or this same table
    // for a self-reference; the added column is the FK's CHILD, looked up in the
    // post-ADD column set because it is not in the catalog yet, and a self-reference's
    // referenced list is looked up there too, so naming the added column finds it. The
    // covering key is still the catalog's, which holds no index of the added column's
    // (`EDGES.md` row 195's residual).
    let referencing_columns: Vec<Column> = {
        let mut cols = get_table_or_err(schema, table_name)?.columns.clone();
        cols.push(new_column.clone());
        cols
    };
    let resolved_fk = foreign_key
        .zip(fk_name)
        .map(|(fk, name)| {
            resolve_and_validate_foreign_key(schema, table_name, fk, name, &referencing_columns)
        })
        .transpose()?;
    // Whether 18.6 rewrites the table for this column — a per-row volatile default —
    // which decides where the row checks, the index build and the FK's type test fall
    // relative to one another (each placement below says which side it is on).
    let volatile_per_row = is_per_row_volatile(new_column.default.as_ref());
    // The FK's column types (`42804`). 18.6 tests them as it adds the constraint, in the
    // pass after the column's and its indexes'; with a rewrite queued that is ahead of
    // the rewrite's `NOT NULL` and `CHECK` verdicts and of the index build
    // (`ATExecAddIndex` defers the build to the rewrite), so it goes here, ahead of the
    // sequence kind gate whose error 18.6 raises in the rewrite. Without one the build has already run — its `54000` and
    // `23505` outrank this — and the test sits after it, below.
    if volatile_per_row && let Some(fk) = &resolved_fk {
        let parent = get_table_or_err(schema, &fk.parent_table)?;
        validate_foreign_key_types(
            fk,
            &referencing_columns,
            foreign_key_referenced_columns(
                &parent.columns,
                &fk.parent_table,
                table_name,
                &referencing_columns,
            ),
        )?;
    }

    // The kind gate on the user's `DEFAULT nextval('x')`, after the naming and the FK
    // and before the backfill — which is exactly where Postgres's own `42809` lands on
    // this route. A volatile default forces a table rewrite, so on a **non-empty** table
    // 18.6 evaluates `nextval` per row and raises `cannot open relation "x"` at the
    // `ALTER` itself; measured, that error loses to a colliding index name (`42P07`) and
    // to a missing FK parent (`42P01`) — both settled in the pass phase — and beats the
    // backfill's own `23514`, since the rewrite computes each row's value before checking
    // its constraints. pglet's gate is unconditional (it stands in for the first write,
    // whenever that comes), which on an *empty* table is the documented over-rejection.
    for seq in &user_default_sequences {
        ensure_sequence_relation(schema, seq)?;
    }

    // Snapshot the existing rows once (read-only) — needed both to backfill and to
    // validate the backfilled values against the new column's constraints.
    let existing: Vec<(RowId, Vec<Value>)> = storage.scan(table_name)?;

    // **Stage** the sequence this statement creates for a SERIAL / identity / user
    // `nextval` default, rather than registering it: it is a catalog mutation, and
    // fallible steps still stand between here and the mutation block — the
    // backfill's own resolution, the `23502` gate, the index build's checks
    // (`validate_added_column_uniques` between its size refusals), the CHECK and FK
    // validations (`validate_added_column_checks`, `validate_added_column_fk`), and
    // the page-bound pre-flight over the back-filled keys.
    // The last was written into this list as an absence rather than a member: a key
    // the build refused was refused *after* the merge, inside `create_unique_index`'s
    // back-fill, which left the sequence durable **and** put the column and its index
    // in the catalog while storage rolled back — the wire-brick shape, not merely a
    // phantom sequence. Registering
    // first left the three violations that gate raises over such a column
    // (`23514` / `23505` / `23503`) with a **durable phantom sequence** on the
    // autocommit path (storage rolls back, `self.schema` does not), carrying the
    // backfill's advances: `ALTER TABLE t ADD COLUMN c SERIAL CHECK (c > 100)` over
    // two rows answered `23514` and then served `nextval('t_c_seq')` → 3, where 18.6
    // answers `42P01` and leaves the name free. The `23502` gate is a step and not a
    // fourth violation: a `NextVal` default resolves to an `int8` that no coercion
    // turns into NULL, so `ADD COLUMN c SERIAL NOT NULL` succeeds on both engines.
    //
    // The no-phantom property is pinned by `pglet`'s
    // `a_failed_add_column_backfill_leaves_no_phantom_sequence` (the implicit-sequence
    // arm) and `a_failed_add_column_leaves_no_phantom_user_named_sequence` (the user
    // `DEFAULT nextval('x')` arm), each asserting the absent catalog entry, the `42P01`
    // a read of it answers, and that the name is free for a later `CREATE SEQUENCE`.
    // No fixture can carry either: a fixture has one `@test` whose last statement's
    // rows are the comparison, so it cannot fail a statement and *then* probe the
    // catalog — an `@expect-error` fixture stops at the SQLSTATE. (The isolating
    // `BEGIN … ROLLBACK` would hide the residue on top of that, but it is not the
    // reason, and `@no-txn` fixtures do not get the wrapper at all.)
    //
    // The record is seeded exactly as the registration seeded it, so the backfill draws
    // the same values (identity `START WITH n` starts at `n`; a plain SERIAL at 1) and
    // the counter that lands carries them. An implicit one is owned by the column it
    // feeds; a name the user wrote in a `DEFAULT nextval('x')` creates no ownership
    // edge.
    //
    // **A sequence the catalog already holds is not staged.** The backfill's draws on it
    // are `nextval`s, which are non-transactional and are *not* rolled back
    // (`Schema::restore_from`'s exception), so an existing record is advanced **in
    // place** rather than through a copy a failure would discard. Keeping the two arms'
    // `contains_key` states that locally, at the one place a `Sequence` record is
    // minted: nothing downstream has to be consulted to know that a staged record is a
    // record no other code owns.
    //
    // **Both are belts, and today both are dead — measured, not assumed.** Removing
    // *both* is green across `cargo test --workspace` and the whole fixture corpus as
    // of the measuring run (1721 fixtures then; a count here only goes stale), because
    // three later facts already make a live name's staged copy unreachable: `working`
    // overwrites it with the live record on the very next lines, the write-back moves
    // `last_value` onto the live record either way, and the merge below is an
    // `or_insert` rather than an `insert`. They are kept as belts because that argument
    // is three-deep and the guard is one line; do not re-derive it as coverage.
    //
    // **The guard that is load-bearing here is the write-back's *position*.** It sits
    // above, before the `?` on `backfill_result`, so a *failed* statement still leaves
    // the live counter where its draws left it. Moving that loop down to the merge point
    // reddens exactly one assertion under `cargo test --workspace` — the `nextval` → 3
    // at the end of `a_failed_add_column_leaves_no_phantom_user_named_sequence`, over a
    // `live_addcol_seq` the test creates first (it answers 1). That single assertion is
    // the whole coverage of the in-place path, and the fixture corpus was not measured
    // under that mutation because it structurally cannot hold it: no fixture combines an
    // `ADD COLUMN` with a pre-created sequence and then observes the counter, since a
    // fixture's one `@test` is compared on its last statement's rows and an
    // `@expect-error` stops at the SQLSTATE.
    let mut staged_sequences: BTreeMap<String, Sequence> = BTreeMap::new();
    if let Some(seq) = &implicit_sequence {
        if !schema.sequences.contains_key(seq) {
            let start = column.identity_start.unwrap_or(1);
            staged_sequences.insert(
                seq.clone(),
                Sequence::owned_by(start, table_name, &column.name),
            );
        }
    } else {
        // The user's own `DEFAULT`, through the one sequence-reference derivation, so
        // an expression default stages **every** sequence it creates rather than only
        // the bare `nextval` spelling's one.
        for seq in &user_default_sequences {
            if !schema.sequences.contains_key(seq) {
                staged_sequences.insert(seq.clone(), Sequence::new(1));
            }
        }
    }

    // The map the backfill draws from: the staged records **plus a copy of the live
    // record of every sequence this column's default already names**.
    //
    // The two used to be selected between (`if staged.is_empty()`), on the argument
    // that they are disjoint — true while a default named exactly one sequence, and
    // false the moment an expression can name several: over a non-empty table,
    // `DEFAULT nextval('<live>') + nextval('<new>')` needs both maps at once, and
    // either selection answers `42P01` for the sequence the other one holds. The guard
    // is `pglet`'s `an_add_column_default_may_draw_a_live_and_a_fresh_sequence_at_once`,
    // and it has to be a unit test: a differential fixture must `CREATE` both sequences
    // for 18.6, which puts both on the live path and makes the selection green.
    //
    // The **write-back below is what keeps the live half in-place**, and it is the
    // reason a live record may be copied here at all: `nextval` is non-transactional,
    // so an advance on an existing sequence stands whether this statement succeeds or
    // fails, and a copy silently discarded on error would walk a live counter back
    // over ids already issued. Only `last_value` moves (a counter update is not a
    // record overwrite — `start` and `owner` are the sequence's own facts).
    let drawn_sequences: Vec<String> = new_column
        .default
        .as_ref()
        .map(default_sequence_refs)
        .unwrap_or_default();
    let mut working: BTreeMap<String, Sequence> = staged_sequences.clone();
    for seq in &drawn_sequences {
        if let Some(live) = schema.sequences.get(seq) {
            working.insert(seq.clone(), live.clone());
        }
    }

    // Resolve the backfill value for every existing row. A **stable** default is
    // evaluated once and reused; a **volatile** one is resolved per row — see
    // [`is_per_row_volatile`].
    let backfill_result: Result<(Option<Value>, Vec<Value>)> = (|| {
        let sequences = &mut working;
        let uniform_value = if existing.is_empty() || volatile_per_row {
            None
        } else {
            Some(resolve_default(&new_column, sequences)?)
        };
        // **Each back-filled value is a write into the column**, and the write path's
        // member test is the one it skips: an enum column took whatever its default
        // resolved to — `now()`, `gen_random_uuid()`, `nextval(…)`, `'q' || ''` — each
        // `42804` on 18.6 (`EDGES.md` row 6) and stored here as a label the enum does not
        // hold (row 105). Refused as the enum's `22P02`, loud under the wrong code, before
        // any row is written; the cook refuses what it evaluates earlier, at its own
        // position, and this is the test every arm's value passes.
        let backfill = existing
            .iter()
            .map(|_| {
                let value = uniform_value.as_ref().map_or_else(
                    || resolve_default(&new_column, sequences),
                    |v| Ok(v.clone()),
                )?;
                super::constraints::validate_enum_member(&new_column, &value, &schema.enums)?;
                Ok(value)
            })
            .collect::<Result<Vec<_>>>()?;
        Ok((uniform_value, backfill))
    })();
    // Unconditionally, and **before** the `?`: a live sequence's advances are not this
    // statement's to roll back. A staged one's are discarded with the statement, which
    // is the whole point of staging — so the reconciliation is by which map the name
    // came from, not by whether the backfill succeeded.
    for (name, advanced) in &working {
        if let Some(live) = schema.sequences.get_mut(name) {
            live.last_value = advanced.last_value;
        }
    }
    for (name, staged) in &mut staged_sequences {
        if let Some(advanced) = working.get(name) {
            staged.last_value = advanced.last_value;
        }
    }
    let (uniform_value, backfill) = backfill_result?;

    // A NOT NULL column whose backfill is NULL can't fill the existing rows
    // (Postgres `23502 column "c" of relation "t" contains null values`, raised in
    // `ATRewriteTable`). **The test is the resolved values, not the presence of a
    // DEFAULT**: reading `default.is_none()` skipped `DEFAULT NULL`, whose
    // `Some(Literal(Null))` resolves to NULL just the same, and pglet wrote NULLs into
    // the NOT NULL column — after which an index over it made an accelerated-profile
    // `ORDER BY` read (0 rows). An empty table has no backfill and so no violation,
    // which is 18.6's answer too (measured: the same `ALTER` over zero rows succeeds).
    //
    // **Be exact about what the values form buys today: nothing that is measurable.**
    // `Parser::parse_column_default` normalizes `DEFAULT NULL` to `None`, so the two
    // formulations are equivalent over every default SQL can currently write —
    // measured as a mutation, reverting the no-rewrite gate's test (the `!volatile_per_row`
    // block below the index build) to the presence test reddens no fixture and no test,
    // as it did where that gate stood above the build. It is kept because it is the statement `ATRewriteTable`
    // itself makes (it inspects the rewritten tuples, not the catalog default) and
    // because it is the form that survives a `DEFAULT <expr>` whose *value* decides.
    // Removing the gate outright is what the guards see: three unit tests and
    // `ddl/add_column_not_null_default_null_is_23502.sql` +
    // `ddl/alter_addcol_notnull_no_default_loud.sql`.
    //
    // Placed **after** the backfill it judges and **before** the CHECK, which is 18.6's
    // order: `ATRewriteTable` checks the not-null attributes (`tablecmds.c:6465`)
    // before the constraint list (`:6502`) — for each row in turn, which only a
    // per-row default can tell from a whole-table pass, and which is therefore the
    // rewrite branch's (`validate_added_column_checks`, below). The discriminating statement is `NOT NULL DEFAULT NULL CHECK (c IS
    // NOT NULL)` — `23502` on both engines in either textual order, against `23514`
    // for the same CHECK without the NOT NULL. The obvious `CHECK (c > 100)` spelling
    // proves nothing here: a NULL makes that predicate NULL, which is no violation, so
    // the `23502` wins wherever this gate sits. Everything that outranks it on 18.6 —
    // the column type's `42704`, a declared constraint name's `42710`, a missing FK
    // parent's `42P01`, and the FK's column types' `42804` — is settled ahead of it
    // (each measured on both engines).
    //
    // **Where the whole row check sits depends on whether 18.6 rewrites the table**, and
    // it rewrites exactly when the default is per-row volatile ([`is_per_row_volatile`]).
    // With a rewrite the index is not built as its subcommand runs — `ATExecAddIndex`
    // skips the build when a rewrite is queued — but rebuilt after the rewritten rows,
    // each checked NOT NULL then CHECK as it is written, so every row check precedes the
    // whole build. Without one the build runs as its subcommand does, then the FK's
    // type test as that constraint is added, and the row checks after both, in the
    // verification scan — so on that branch the NOT NULL gate sits below the build and
    // the type test (the `!volatile_per_row` block after them). Against the build alone
    // its position is unobservable, the backfill being one value that is NULL for every
    // row or for none, and a NULL entry failing no build check; against the type test it
    // is measured: `ADD COLUMN a text NOT NULL UNIQUE REFERENCES p` over two rows, `p`'s
    // key an `int`, is `42804` on 18.6, where this gate above the build answered `23502`.
    // Measured on 18.6 over two
    // rows: `ADD COLUMN c text UNIQUE CHECK (length(c) < 10) DEFAULT
    // (gen_random_uuid()::text || '<3 000 characters>')` is `23514`, and the same
    // column with a constant default is the build's `54000`; `ADD COLUMN b int UNIQUE
    // CHECK (b < 0) DEFAULT (length(gen_random_uuid()::text) - 31)` is `23514` and its
    // `DEFAULT 5` spelling the build's `23505`; `ADD COLUMN b int NOT NULL CHECK (b > 1)
    // DEFAULT NULLIF(nextval('s'), 2)` is the first row's `23514` ahead of the second
    // row's NULL, and `NULLIF(nextval('s'), 1)` the first row's `23502`. The foreign key
    // is checked after the build on both branches (a final pass over the table): with
    // `UNIQUE REFERENCES p` and no parent row, the per-row default's duplicate is
    // `23505` there.
    if volatile_per_row {
        validate_added_column_checks(
            schema,
            table_name,
            &column.name,
            new_column.type_oid,
            new_column.enum_type.as_deref(),
            &named_checks,
            !column.nullable,
            &existing,
            &backfill,
        )?;
    }

    // The index each inline UNIQUE/PRIMARY KEY builds, over the back-filled values —
    // ahead of the FK's row check on both branches and, on the no-rewrite one, of the
    // FK's type test, the NOT NULL gate and the CHECK (below).
    // Inside the build its refusals run as a `CREATE INDEX`'s do (`scan_index_build_sizes`):
    // the tuple-size field as the rows are read, the duplicate from the sort, the item
    // limit as the sorted entries are loaded. The column is the whole key, so an entry's
    // size is its value's; a build over one repeated default cannot tell load order from
    // read order, and a per-row default is read in row order.
    let is_enum = new_column.enum_type.is_some();
    let entry_sizes: Vec<Option<usize>> = backfill
        .iter()
        .map(|value| super::index_row::index_entry_size([(value, is_enum)]))
        .collect();
    if !column_uniques.is_empty() {
        for &size in &entry_sizes {
            super::index_row::check_entry_size_field(size)?;
        }
    }
    validate_added_column_uniques(
        table_name,
        &column.name,
        new_column.type_oid,
        &index_names,
        &backfill,
    )?;
    if let Some(index_name) = index_names.first() {
        for &size in &entry_sizes {
            super::index_row::check_btree_item_size(size, index_name, table_name)?;
        }
    }

    // The no-rewrite branch's FK type test, then its verification scan — the NOT NULL
    // gate and the inline CHECKs over the backfilled rows — and on both branches the FK
    // over them, still before any mutation. The type test is below the build here and
    // above it on the rewrite branch (`validate_foreign_key_types`'s placement, at the
    // resolution above): `ADD COLUMN a text DEFAULT 'q' UNIQUE REFERENCES p` over two rows,
    // `p`'s key an `int`, is the build's `23505` on 18.6, and its per-row-volatile
    // spelling the `42804`.
    if !volatile_per_row {
        if let Some(fk) = &resolved_fk {
            let parent = get_table_or_err(schema, &fk.parent_table)?;
            validate_foreign_key_types(
                fk,
                &referencing_columns,
                foreign_key_referenced_columns(
                    &parent.columns,
                    &fk.parent_table,
                    table_name,
                    &referencing_columns,
                ),
            )?;
        }
        if !column.nullable && backfill.iter().any(Value::is_null) {
            return Err(Error::NotNullColumnHasNulls {
                table: table_name.to_string(),
                column: column.name.clone(),
            });
        }
        validate_added_column_checks(
            schema,
            table_name,
            &column.name,
            new_column.type_oid,
            new_column.enum_type.as_deref(),
            &named_checks,
            false,
            &existing,
            &backfill,
        )?;
    }
    validate_added_column_fk(
        schema,
        storage,
        table_name,
        &column.name,
        new_column.type_oid,
        resolved_fk.as_ref(),
        &backfill,
    )?;

    // pglet's own over-rejection, and so the **last** fallible step: its page bound on
    // the keys the index build below encodes, ahead of the mutation block, whose build
    // would otherwise raise it with the column already appended.
    if let Some(index_name) = index_names.first() {
        for value in &backfill {
            super::index_row::check_page_key_len(
                super::index_row::encoded_key_len(std::slice::from_ref(value), false),
                index_name,
            )?;
        }
    }

    // The other pglet-only over-rejection, and so beside it rather than up in the
    // cook: the render probe on every inline CHECK, once the naming, the FK, the
    // sequence kind gate, the `23502` gate and the backfill validation have all had
    // their say. Riding inside `validate_check_exprs` it pre-empted this route's
    // duplicate CHECK name (`42710`), missing FK parent (`42P01`) and backfill
    // violation (`23514`) with a `0A000` — see `validate_stored_expr_renders`. The
    // index-name collision was never pre-empted (it is minted above the CHECK loop),
    // which is the control that identifies the position rather than the refusal.
    validate_check_exprs_render(&named_checks)?;

    // All checks passed — mutate. The staged sequence lands first, which is
    // `transformAlterTableStmt`'s own order (the `CreateSeqStmt` rides the before-list,
    // ahead of the column) and the order the naming above already assumed. `or_insert`
    // rather than a plain insert, matching what the registration did: the staged name
    // is free by construction on both arms, and nothing since has touched the map.
    for (name, sequence) in std::mem::take(&mut staged_sequences) {
        schema.sequences.entry(name).or_insert(sequence);
    }

    // Append the column to the catalog.
    if let Some(table) = schema.get_table_mut(table_name) {
        table.columns.push(new_column);
    }

    // Backfill storage. A uniform value grows every row in one pass; a volatile
    // per-row value rewrites each row with its own value appended.
    if volatile_per_row {
        for ((id, row), value) in existing.iter().zip(&backfill) {
            let mut new_row = row.clone();
            new_row.push(value.clone());
            storage.update(table_name, *id, new_row)?;
        }
    } else {
        let fill = uniform_value.unwrap_or(Value::Null);
        durable_write_failure(storage.add_column(table_name, fill))?;
    }

    // Record the (already-validated) inline FK so the standard `enforce_foreign_keys`
    // (write path) and `execute_delete`'s `ON DELETE` engine cover future writes.
    if let Some(fk) = resolved_fk
        && let Some(table) = schema.get_table_mut(table_name)
    {
        table.foreign_keys.push(fk);
    }

    // Record the inline CHECK constraints under the names minted above — enforced on
    // every future write, visible in `information_schema.table_constraints`.
    if !named_checks.is_empty()
        && let Some(table) = schema.get_table_mut(table_name)
    {
        table.checks.extend(named_checks);
    }

    // Build an enforced unique index per inline UNIQUE/PRIMARY KEY (over the
    // now-backfilled rows) — the same enforcement a `CREATE TABLE` inline UNIQUE/PK
    // creates. A PK also flips its column NOT NULL (already the case here, since the
    // parser marks a `PRIMARY KEY` column non-nullable).
    for (uc, index_name) in column_uniques.iter().zip(&index_names) {
        create_unique_index(
            schema,
            storage,
            index_name,
            table_name,
            &uc.columns,
            constraint_kind_of(uc),
        )?;
    }

    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// Whether an `ALTER TABLE … ADD COLUMN` backfill must re-resolve `default` for
/// **each** existing row rather than evaluating it once and copying the value.
///
/// Postgres decides this by the default expression's volatility: a `VOLATILE` function
/// forces a table rewrite that calls it per row, while a `STABLE` one is called once
/// per statement and an `IMMUTABLE` constant is not even written into the rows (it goes
/// into `pg_attribute.attmissingval` — measured: `DEFAULT 42` leaves `atthasmissing`
/// true and `{42}`, while every member of the `now` family leaves it false, so those
/// all rewrite).
///
/// pglet's closed `ColumnDefault` set maps onto that as three volatile members:
///
/// - `RandomUuid` — a fresh UUID per row.
/// - `NextVal` — a fresh sequence value per row.
/// - `ClockTimestamp` — a fresh clock read per row. This is the member the collapsed
///   `now`-family representation hid: with all five spellings stored as one variant,
///   `ADD COLUMN c timestamptz DEFAULT clock_timestamp()` over five rows wrote **one**
///   instant into all five where 18.6 writes several (measured on one idle host: 3 or 4
///   distinct over 5 rows, 6–11 over 20 — never all-distinct there, because
///   consecutive calls can share a microsecond).
///
/// The other four `now`-family spellings are **not** volatile for this purpose:
/// `now()` and `transaction_timestamp()` are transaction-stable and
/// `statement_timestamp()` / `CURRENT_TIMESTAMP` statement-stable, and one `ALTER` is
/// one statement inside one transaction, so 18.6 backfills a single value for each
/// (measured: `count(DISTINCT) = 1` over five rows for all four). `Literal`,
/// `CurrentDate`, `CurrentTime` and `None` are constant per statement as well.
///
/// (A multi-row `INSERT` makes the same split by another route: its rows resolve the
/// default one at a time, and the four read a clock that does not move inside the
/// statement — `crate::builtins`.)
/// An [`ColumnDefault::Expr`] is volatile when it **contains** any of them, which is
/// the one arm that has to be a walk rather than a match: `DEFAULT nextval('s') + 1`
/// and `DEFAULT clock_timestamp() + interval '1 day'` are as volatile as their bare
/// spellings, and reading only the top node would backfill one value into every row.
fn is_per_row_volatile(default: Option<&ColumnDefault>) -> bool {
    match default {
        Some(
            ColumnDefault::RandomUuid | ColumnDefault::NextVal(_) | ColumnDefault::ClockTimestamp,
        ) => true,
        // A node is per-row volatile where 18.6 marks its call `VOLATILE`
        // (`Expr::is_volatile_call`): `gen_random_uuid()`, `clock_timestamp()` and a
        // sequence function, the three the closed arms name, and `random()` and
        // `pg_advisory_xact_lock`, which reach a default only as an expression. The
        // `now`-family builtins other than `clock_timestamp()` and
        // `CURRENT_DATE`/`CURRENT_TIME` are statement-stable and so are *not* — the split
        // a "contains any impure builtin" test would get wrong in the direction that
        // costs a table rewrite for nothing.
        Some(ColumnDefault::Expr(expr)) => expr.contains_volatile_call(),
        _ => false,
    }
}

/// The `ALTER TABLE ADD COLUMN` inline `UNIQUE`|`PRIMARY KEY` indexes' duplicate check
/// over the backfilled values — the index build's `23505`, which 18.6 raises before the
/// column's FK ([`validate_added_column_fk`]) and, when it does not rewrite the table,
/// before its CHECK ([`validate_added_column_checks`]).
fn validate_added_column_uniques(
    table_name: &str,
    new_column_name: &str,
    new_column_type: TypeOid,
    unique_index_names: &[String],
    backfill: &[Value],
) -> Result<()> {
    // UNIQUE / PRIMARY KEY (single-column, on the new column): the backfilled values
    // must be distinct (NULLs are distinct). A constant DEFAULT over >1 row collides.
    // The index names are the caller's already-minted ones, so the `23505` names the
    // constraint that will actually exist (an auto-name may have been disambiguated).
    for index_name in unique_index_names {
        // Identity → the **first** backfilled value under it, which is the one 18.6's
        // DETAIL names.
        let mut seen: HashMap<Vec<u8>, &Value> = HashMap::new();
        for value in backfill {
            if value.is_null() {
                continue; // NULLs are distinct — not a uniqueness candidate.
            }
            // The index's own equality, the same function the `CREATE UNIQUE INDEX`
            // build gate and `validate_existing_unique` key through — the third of the
            // three validation scans, and it had the same structural basis they did.
            let identity = super::constraints::index_key_identity(std::slice::from_ref(value))?;
            match seen.entry(identity) {
                std::collections::hash_map::Entry::Occupied(first) => {
                    return Err(Error::DuplicateIndexKey {
                        index: index_name.clone(),
                        detail: Some(Box::new(pglet_core::UniqueViolationDetail {
                            table: table_name.to_string(),
                            key: Some(pglet_core::KeyDescription::new(
                                vec![new_column_name.to_string()],
                                vec![crate::types::value_to_text_with_oid(
                                    first.get(),
                                    new_column_type,
                                )],
                            )),
                        })),
                    });
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(value);
                }
            }
        }
    }
    Ok(())
}

/// Validate the backfilled rows of an `ALTER TABLE ADD COLUMN` against the new column's
/// inline `CHECK`s, read-only (before the column is added or the rows are grown) — and,
/// with `not_null`, its NOT NULL first, row by row, as 18.6's rewrite checks each row it
/// writes. Its inline `UNIQUE`|`PRIMARY KEY` is [`validate_added_column_uniques`]'s and
/// its `REFERENCES` [`validate_added_column_fk`]'s; the caller orders the three.
///
/// `existing` is the pre-add snapshot (each `(id, row)`); `backfill[i]` is the value
/// the new column takes in row `existing[i]`. A violation is loud — `23502` or `23514`.
#[allow(clippy::too_many_arguments)]
fn validate_added_column_checks(
    schema: &Schema,
    table_name: &str,
    new_column_name: &str,
    new_column_type: TypeOid,
    new_column_enum: Option<&str>,
    checks: &[CheckConstraint],
    not_null: bool,
    existing: &[(RowId, Vec<Value>)],
    backfill: &[Value],
) -> Result<()> {
    if !not_null && checks.is_empty() {
        return Ok(());
    }
    // CHECK: assemble each candidate row (existing row ++ backfilled value) with the
    // new column appended to the name scope, then run the normal CHECK evaluator.
    let (mut col_names, mut col_types): (Vec<String>, Vec<TypeOid>) = {
        let table = get_table_or_err(schema, table_name)?;
        (
            table.columns.iter().map(|c| c.name.clone()).collect(),
            table.columns.iter().map(|c| c.type_oid).collect(),
        )
    };
    col_names.push(new_column_name.to_string());
    col_types.push(new_column_type);
    // The back-filled column's own enum order goes on the end beside its type: the
    // column being added may itself be the enum a CHECK orders.
    let mut col_enums: Vec<Option<&[String]>> = {
        let table = get_table_or_err(schema, table_name)?;
        super::helpers::table_enum_orders(&table.columns, &schema.enums)
    };
    col_enums.push(super::helpers::enum_order_of(
        &schema.enums,
        new_column_enum,
    ));
    let col_refs: Vec<&str> = col_names.iter().map(String::as_str).collect();
    let prepared: Vec<(String, Expr)> = checks.iter().map(prepare_check).collect();
    for ((_, row), value) in existing.iter().zip(backfill) {
        if not_null && value.is_null() {
            return Err(Error::NotNullColumnHasNulls {
                table: table_name.to_string(),
                column: new_column_name.to_string(),
            });
        }
        if prepared.is_empty() {
            continue;
        }
        let mut candidate = row.clone();
        candidate.push(value.clone());
        enforce_checks(
            table_name, &prepared, &col_refs, &col_types, &col_enums, &candidate,
        )
        .map_err(as_validation_failure)?;
    }
    Ok(())
}

/// Validate the backfilled values of an `ALTER TABLE ADD COLUMN` against the new
/// column's inline `REFERENCES`, read-only: every non-NULL value must reference a live
/// parent (MATCH SIMPLE exempts a NULL) — `23503` otherwise. 18.6 checks it in a final
/// pass over the table, after the column's index build and row checks.
fn validate_added_column_fk<S: Storage>(
    schema: &Schema,
    storage: &S,
    table_name: &str,
    new_column_name: &str,
    new_column_type: TypeOid,
    resolved_fk: Option<&ForeignKey>,
    backfill: &[Value],
) -> Result<()> {
    // Single-column (the inline ADD-COLUMN FK).
    if let Some(fk) = resolved_fk {
        for value in backfill {
            if value.is_null() {
                continue;
            }
            if !parent_tuple_exists(
                storage,
                schema,
                &fk.parent_table,
                &fk.parent_columns,
                std::slice::from_ref(value),
            )? {
                return Err(Error::ForeignKeyViolation(Box::new(
                    pglet_core::ForeignKeyViolationDetail {
                        child_table: table_name.to_string(),
                        parent_table: fk.parent_table.clone(),
                        constraint: fk.name.clone(),
                        direction: pglet_core::FkDirection::Insert,
                        key: pglet_core::KeyDescription::new(
                            vec![new_column_name.to_string()],
                            vec![crate::types::value_to_text_with_oid(value, new_column_type)],
                        ),
                    },
                )));
            }
        }
    }
    Ok(())
}

/// Re-key a write-path `23514` as the **validation-scan** one.
///
/// The two `ALTER TABLE` routes that re-check existing rows — `ADD COLUMN` with an
/// inline `CHECK`, and `ADD CONSTRAINT … CHECK` — reach the same `enforce_checks`, but
/// 18.6 reports them from `ATRewriteTable`, not `ExecConstraints`: `check constraint
/// "x" of relation "y" is violated by some row`, with **no** `Failing row contains`
/// detail (there is no new row to show). Both measured on 18.6. Mapping here rather
/// than duplicating the evaluator keeps `enforce_checks` single-homed.
fn as_validation_failure(e: Error) -> Error {
    match e {
        Error::CheckViolation {
            table, constraint, ..
        } => Error::CheckConstraintNotSatisfied { table, constraint },
        other => other,
    }
}

/// Execute ALTER TABLE ADD CONSTRAINT ... CHECK.
///
/// Type-checks the predicate, mints the constraint's name, validates every existing
/// row against it (Postgres rejects an `ADD CONSTRAINT CHECK` that current data
/// violates), then records the check on the table — after which the standard
/// write-path `enforce_checks` covers every future write and
/// `information_schema.table_constraints` lists it. The catalog is left untouched if
/// any of that fails.
pub fn execute_alter_table_add_check<S: Storage>(
    schema: &mut Schema,
    // Read-only here — existing rows are scanned to validate the new predicate; the
    // constraint is recorded on `schema`, not `storage`.
    storage: &S,
    table_name: &str,
    check: &CheckConstraintDef,
) -> Result<QueryResult> {
    // Column order (and table existence + kind) in one immutable borrow, released
    // before the mutable push below. This is the statement's **first** relation
    // lookup, so it is the one that decides the missing/wrong-kind answer — the
    // `get_table_or_err` further down is dead for a name this one rejects.
    let (col_names, col_types): (Vec<String>, Vec<TypeOid>) = {
        let table = get_table_or_err(schema, table_name)?;
        (
            table.columns.iter().map(|c| c.name.clone()).collect(),
            table.columns.iter().map(|c| c.type_oid).collect(),
        )
    };

    // Type-check the predicate's operands first: an empty (or all-satisfying) table
    // makes the row scan below prove nothing, so without this an
    // `ADD CONSTRAINT … CHECK (int_col LIKE '1%')` would be accepted and then break
    // every future insert. Postgres rejects it here, `42883` — and reports it *ahead*
    // of a duplicate name on the same constraint, because `AddRelationNewConstraints`
    // cooks each constraint before naming it (measured on 18.6:
    // `ADD CONSTRAINT dup CHECK (int_col LIKE 'x')` against an existing `dup` is
    // `42883`, not `42710`).
    {
        let table = get_table_or_err(schema, table_name)?;
        validate_check_exprs(std::slice::from_ref(check), &table.columns, &schema.enums)?;
    }

    // Mint the name: a declared one is gated `42710` (the pool spans all four kinds, so
    // an existing `PRIMARY KEY`/`UNIQUE` of that name collides too), an omitted one is
    // generated and disambiguated. Storing the generated name is what makes the
    // violation message, `information_schema`, and the dump read one fact instead of
    // three independently-synthesized ones.
    let named = CheckConstraint {
        name: ConstraintNames::new(schema, table_name).mint_check(check)?,
        expr: check.expr.clone(),
    };

    // Validate existing rows against the new predicate before recording it.
    let parsed = prepare_check(&named);
    let col_refs: Vec<&str> = col_names.iter().map(String::as_str).collect();
    let col_enums = {
        let table = get_table_or_err(schema, table_name)?;
        super::helpers::table_enum_orders(&table.columns, &schema.enums)
    };
    for (_id, row) in storage.scan(table_name)? {
        enforce_checks(
            table_name,
            std::slice::from_ref(&parsed),
            &col_refs,
            &col_types,
            &col_enums,
            &row,
        )
        .map_err(as_validation_failure)?;
    }

    // pglet's own over-rejection, last: the predicate must **render**, or this one
    // constraint takes the whole database's dump down. 18.6 raises nothing for it, so
    // it must lose to every gate above — measured, inside the cook it answered `0A000`
    // where 18.6 reports the duplicate constraint name (`42710`) and the existing-row
    // violation (`23514`). This route carries one CHECK and so has no reverse
    // arrangement at all, which is why only the position could be measured here.
    validate_check_exprs_render(std::slice::from_ref(&named))?;

    // Record it — enforced on every future write, visible in table_constraints.
    if let Some(table) = schema.get_table_mut(table_name) {
        table.checks.push(named);
    }
    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// Execute `ALTER TABLE … ADD CONSTRAINT … { PRIMARY KEY | UNIQUE | FOREIGN KEY }`.
///
/// Builds the **same** enforcement a `CREATE TABLE` inline constraint would — an
/// auto-created enforced unique index for PK/UNIQUE, a recorded `ForeignKey` for FK
/// — after validating that the table's existing rows already satisfy it. A violation
/// fails loud (`23505` duplicate / `23502` NULL-in-PK / `23503` orphan child) and
/// leaves the catalog untouched, so a restored database enforces the constraint
/// exactly as the original did. `CHECK` is handled by `execute_alter_table_add_check`.
pub fn execute_alter_table_add_constraint<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    table_name: &str,
    constraint: &AddConstraint,
) -> Result<QueryResult> {
    match constraint {
        AddConstraint::PrimaryKey { columns, name } => {
            add_primary_key(schema, storage, table_name, columns, name.as_deref())
        }
        AddConstraint::Unique { columns, name } => {
            add_unique(schema, storage, table_name, columns, name.as_deref())
        }
        AddConstraint::ForeignKey(fk) => add_foreign_key(schema, storage, table_name, fk),
    }
}

/// `ADD CONSTRAINT … PRIMARY KEY (cols)`: reject a second primary key, validate
/// the index build over the existing rows and then that they are non-NULL, set the
/// columns NOT NULL, then create the enforced index under the declared name (else a
/// disambiguated `<table>_pkey`).
///
/// The order is Postgres's, measured on 18.6: a repeated key column is `42701`
/// first — `transformIndexConstraint` tests it in parse analysis, and on this route
/// skips the resolution that would otherwise precede it — then the columns are
/// resolved (`42703`, via the not-null constraints the parser queues ahead of the
/// index), then `index_check_primary_key`'s `42P16`, then `index_create`'s name
/// checks (`42P07` / `42710`), and only then the data — the build's `54000` / `23505`,
/// then the `23502`.
fn add_primary_key<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    table_name: &str,
    columns: &[String],
    declared_name: Option<&str>,
) -> Result<QueryResult> {
    let uc = UniqueConstraint {
        columns: columns.to_vec(),
        is_primary: true,
        name: declared_name.map(ToString::to_string),
    };
    let index_name = {
        let table = get_table_or_err(schema, table_name)?;
        // Parse analysis, so it outranks the checks below it here — including the column's
        // own `42703`, measured: `ALTER TABLE t ADD CONSTRAINT p PRIMARY KEY (nope,
        // nope)` is `42701` on 18.6, where the `CREATE TABLE` spelling is `42703`.
        // It stays behind the **relation's** `42P01`, which `AlterTable` raises
        // before any subcommand is transformed (`ALTER TABLE nosuchtbl ADD CONSTRAINT
        // u UNIQUE (a, a)` is `42P01`, measured).
        validate_key_columns(table_name, None, &uc)?;
        validate_columns_exist(table, columns)?;
        ensure_no_primary_key(schema, table_name)?;
        ConstraintNames::new(schema, table_name).mint_constraint_index(&uc)?
    };
    {
        let table = get_table_or_err(schema, table_name)?;
        // PK requires entries its index can hold (`54000`), unique key tuples (`23505`)
        // and non-NULL key columns (`23502`) — every check ahead of the `NOT NULL` flip
        // below, the first catalog write. In 18.6's order: the index is built first, as
        // a `UNIQUE`'s is (the size field as the rows are read, the duplicate from the
        // sort — where a NULL key is distinct — the item limit as the entries load), and
        // the columns' NOT NULL is verified after it, by the table scan `ATRewriteTable`
        // runs in the later phase. Measured on 18.6 over rows `(NULL), ('<3 000
        // characters>')` (`54000 index row size 3016`), `('x'), ('x'), ('<9 000
        // characters>')` (`index row requires 9016 bytes`) and `(NULL), ('x'), ('x')`
        // (`23505`).
        let mut sizes = constraint_index_build_sizes(storage, schema, table, columns, &index_name)?;
        sizes.raise_before_duplicates()?;
        validate_existing_unique(storage, table, columns, &index_name)?;
        sizes.raise_after_duplicates()?;
        validate_existing_not_null(storage, table, columns)?;
        sizes.raise_page_bound()?;
    }
    // Set the PK columns NOT NULL (a PK implies NOT NULL).
    if let Some(table) = schema.get_table_mut(table_name) {
        for col in &mut table.columns {
            if columns.iter().any(|c| c == &col.name) {
                col.nullable = false;
            }
        }
    }
    create_unique_index(
        schema,
        storage,
        &index_name,
        table_name,
        columns,
        ConstraintKind::PrimaryKey,
    )?;
    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// `ADD CONSTRAINT … UNIQUE (cols)`: validate existing rows are unique (NULLs are
/// distinct, so a row with a NULL key is exempt), then create the enforced index
/// under the declared name (else a disambiguated `<table>_<cols>_key`).
///
/// There is no de-duplication against a *pre-existing* identical index: `ALTER TABLE
/// t ADD UNIQUE (a)` twice builds `t_a_key` and `t_a_key1` on both engines. Postgres
/// merges redundant specifications only within one statement's own list
/// ([`dedup_unique_constraints`]) — "XXX in ALTER TABLE case, it'd be nice to look
/// for duplicate pre-existing indexes, too."
///
/// A **repeated** key column is refused, on the same terms and at the same position
/// as [`add_primary_key`]'s (see [`validate_key_columns`]).
fn add_unique<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    table_name: &str,
    columns: &[String],
    declared_name: Option<&str>,
) -> Result<QueryResult> {
    let uc = UniqueConstraint {
        columns: columns.to_vec(),
        is_primary: false,
        name: declared_name.map(ToString::to_string),
    };
    let index_name = {
        let table = get_table_or_err(schema, table_name)?;
        validate_key_columns(table_name, None, &uc)?;
        validate_columns_exist(table, columns)?;
        ConstraintNames::new(schema, table_name).mint_constraint_index(&uc)?
    };
    {
        let table = get_table_or_err(schema, table_name)?;
        // The build's size refusals straddle its duplicate check, as 18.6's do
        // (`scan_index_build_sizes`).
        let mut sizes = constraint_index_build_sizes(storage, schema, table, columns, &index_name)?;
        sizes.raise_before_duplicates()?;
        validate_existing_unique(storage, table, columns, &index_name)?;
        sizes.raise_after_duplicates()?;
        sizes.raise_page_bound()?;
    }
    create_unique_index(
        schema,
        storage,
        &index_name,
        table_name,
        columns,
        ConstraintKind::Unique,
    )?;
    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// `ADD CONSTRAINT … FOREIGN KEY (col…) REFERENCES parent(col…)` (single- or
/// multi-column): validate the child columns exist and the parent has a covering
/// unique key (`42830`/`42804`), that every existing non-NULL (MATCH SIMPLE) child
/// tuple references a live parent (`23503` otherwise), then record the FK so the
/// standard write-path / `ON DELETE` machinery covers it.
fn add_foreign_key<S: Storage>(
    schema: &mut Schema,
    storage: &S,
    table_name: &str,
    fk: &ForeignKeyDef,
) -> Result<QueryResult> {
    // Validate child columns exist + resolve/verify the referenced key first. The name
    // is minted before the referenced-key resolution, as Postgres's
    // `ATExecAddConstraint` does (a *declared* duplicate is `42710` ahead of the
    // parent's `42P01`) — and it must happen at all, or two same-named FKs land on one
    // table and a later `DROP COLUMN` sheds the wrong one.
    let resolved = {
        let table = get_table_or_err(schema, table_name)?;
        let name = ConstraintNames::new(schema, table_name).mint_foreign_key(fk)?;
        let resolved =
            resolve_and_validate_foreign_key(schema, table_name, fk, name, &table.columns)?;
        // The column types, before any row is read: 18.6's `42804` outranks the
        // existing rows' `23503`.
        let parent = get_table_or_err(schema, &resolved.parent_table)?;
        validate_foreign_key_types(&resolved, &table.columns, &parent.columns)?;
        resolved
    };
    // Every existing non-NULL child tuple must reference a live parent.
    {
        let table = get_table_or_err(schema, table_name)?;
        let positions: Vec<usize> = resolved
            .columns
            .iter()
            .filter_map(|col| table.columns.iter().position(|c| &c.name == col))
            .collect();
        for (_, row) in storage.scan(table_name)? {
            // MATCH SIMPLE: a NULL in any FK column exempts the row.
            let Some(pair_values) = super::constraints::fk_child_tuple(&row, &positions) else {
                continue;
            };
            if !parent_tuple_exists(
                storage,
                schema,
                &resolved.parent_table,
                &resolved.parent_columns,
                &pair_values,
            )? {
                return Err(Error::ForeignKeyViolation(Box::new(
                    pglet_core::ForeignKeyViolationDetail {
                        child_table: table_name.to_string(),
                        parent_table: resolved.parent_table.clone(),
                        constraint: resolved.name.clone(),
                        direction: pglet_core::FkDirection::Insert,
                        key: pglet_core::KeyDescription::new(
                            resolved.columns,
                            super::constraints::render_key_values(table, &positions, &pair_values),
                        ),
                    },
                )));
            }
        }
    }
    if let Some(table) = schema.get_table_mut(table_name) {
        table.foreign_keys.push(resolved);
    }
    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// Validate every named key column exists in `table` (else `42703`).
fn validate_columns_exist(table: &Table, columns: &[String]) -> Result<()> {
    validate_key_columns_exist(&table.name, &table.columns, columns)
}

/// Validate every named key column exists in `table_columns` (else `42703`).
///
/// The column-slice form of [`validate_columns_exist`] above.
fn validate_key_columns_exist(
    table_name: &str,
    table_columns: &[Column],
    columns: &[String],
) -> Result<()> {
    for c in columns {
        if !table_columns.iter().any(|tc| &tc.name == c) {
            return Err(crate::CatalogError::ColumnNotFound {
                table: table_name.to_string(),
                column: c.clone(),
            }
            .into());
        }
    }
    Ok(())
}

/// Validate that a foreign key's **referenced** column list names each parent column
/// once (`42830`).
///
/// 18.6's `transformFkeyCheckAttrs` refuses the repeat before it looks for a covering
/// index at all, and it is the *referenced* list's rule alone: `FOREIGN KEY (a, a)
/// REFERENCES p (x, y)` is accepted on both engines, so the referencing list's own
/// validator does not call this.
///
/// It runs above the **covering-key** lookup, which is where the position is
/// reachable: `FOREIGN KEY (a, b) REFERENCES p (y, y)` over a parent with no key on
/// `y` is *must not contain duplicates* on 18.6, measured, not *there is no unique
/// constraint matching given keys*. Both are `42830`, so the position decides the
/// message and not the code.
///
/// It runs above the **arity** check too — its callers' rather than its own, run
/// immediately after this returns — and that is **not** what the unequal-arity
/// spelling reports: `FOREIGN KEY (a) REFERENCES p (x, x)` is refused one layer
/// earlier, by the parser's arity check (`parser::ddl`), which also pre-empts the
/// parent's `42P01` — a pre-existing divergence this gate neither causes nor closes.
///
/// An omitted `REFERENCES p` resolves to the parent's PRIMARY KEY, whose own key list
/// this engine already refuses a repeat in ([`validate_key_columns`]); the list is
/// checked here all the same, since both routes reach this function through one
/// `parent_columns` whether it was written or derived.
fn validate_referenced_columns_distinct(parent_columns: &[String]) -> Result<()> {
    for (i, col) in parent_columns.iter().enumerate() {
        if parent_columns[..i].contains(col) {
            return Err(Error::ForeignKeyDuplicateReferencedColumn);
        }
    }
    Ok(())
}

/// Validate one `PRIMARY KEY` / `UNIQUE` constraint's key list — the **constraint's**
/// rule, which its index does not share.
///
/// A repeated key column is `42701` here; the same spelling at `CREATE INDEX t_i ON t
/// (a, a)` and in a foreign key's *referencing* list is accepted by both engines, so
/// neither of those callers routes through this function.
///
/// `table_columns` is `Some` on the `CREATE TABLE` route and `None` on `ALTER TABLE …
/// ADD CONSTRAINT`, which is 18.6's `!cxt->isalter`: `transformIndexConstraint`
/// resolves a key against the statement's own column list and raises `42703` there,
/// and on an `ALTER` it has no such list and leaves the resolution to `DefineIndex`,
/// well below parse analysis. The caller supplies the ALTER route's own `42703` after
/// this returns.
///
/// Each key is tested against the keys **already accepted**, not against the whole
/// list, so on the `CREATE TABLE` route the two verdicts interleave: measured on 18.6,
/// `PRIMARY KEY (a, a, nope)` is `42701` while `PRIMARY KEY (a, nope, a)` and
/// `PRIMARY KEY (nope, a, a)` are `42703`. On the `ALTER` route all three are `42701`,
/// the resolution being absent rather than reordered.
fn validate_key_columns(
    table_name: &str,
    table_columns: Option<&[Column]>,
    uc: &UniqueConstraint,
) -> Result<()> {
    let mut accepted: Vec<&str> = Vec::with_capacity(uc.columns.len());
    for c in &uc.columns {
        if let Some(cols) = table_columns
            && !cols.iter().any(|tc| &tc.name == c)
        {
            return Err(crate::CatalogError::ColumnNotFound {
                table: table_name.to_string(),
                column: c.clone(),
            }
            .into());
        }
        if accepted.contains(&c.as_str()) {
            return Err(crate::CatalogError::DuplicateKeyColumn {
                column: c.clone(),
                primary: uc.is_primary,
            }
            .into());
        }
        accepted.push(c);
    }
    Ok(())
}

/// The size refusals the index an `ADD CONSTRAINT … UNIQUE` / `PRIMARY KEY` builds
/// meets over `table`'s existing rows ([`scan_index_build_sizes`]), keyed through a
/// provisional catalog `Index` of the shape [`create_unique_index`] will build — plain
/// columns, full, `NULLS DISTINCT`, no OID yet.
///
/// [`scan_index_build_sizes`]: crate::executor::constraints::scan_index_build_sizes
fn constraint_index_build_sizes<S: Storage>(
    storage: &S,
    schema: &Schema,
    table: &Table,
    columns: &[String],
    index_name: &str,
) -> Result<crate::executor::constraints::BuildSizeRefusals> {
    let index = Index {
        name: index_name.to_string(),
        table_name: table.name.clone(),
        key: IndexKey::Columns(columns.to_vec()),
        uniqueness: Uniqueness::for_constraint(ConstraintKind::Unique),
        predicate: None,
        nulls_not_distinct: false,
        oid: 0,
    };
    crate::executor::constraints::scan_index_build_sizes(storage, table, &index, &schema.enums)
}

/// Validate that `table`'s existing rows hold no NULL in a would-be primary key's
/// `columns` (`23502`).
///
/// 18.6 verifies the NOT NULL a primary key implies by scanning the table after the
/// index is built, row by row in heap order and, within a row, column by column in the
/// **table's** column order — not the key's: over one row `(NULL, NULL)`, `ADD PRIMARY
/// KEY (b, a)` and `ADD PRIMARY KEY (a, b)` both name `a` there (measured). This scan
/// reads the rows in row-id order, which agrees with the heap's over rows only ever
/// inserted; once an `UPDATE` has moved a row to the heap's end the first NULL row can
/// differ (`EDGES.md` row 194).
fn validate_existing_not_null<S: Storage>(
    storage: &S,
    table: &Table,
    columns: &[String],
) -> Result<()> {
    let positions: Vec<usize> = table
        .columns
        .iter()
        .enumerate()
        .filter(|(_, c)| columns.contains(&c.name))
        .map(|(pos, _)| pos)
        .collect();
    for (_, row) in storage.scan(&table.name)? {
        if let Some(&pos) = positions
            .iter()
            .find(|&&pos| row.get(pos).is_none_or(Value::is_null))
        {
            return Err(Error::NotNullColumnHasNulls {
                table: table.name.clone(),
                column: table.columns[pos].name.clone(),
            });
        }
    }
    Ok(())
}

/// Validate that `table`'s existing rows satisfy a would-be unique constraint over
/// `columns`, before the enforced index is built.
///
/// A row with a NULL key is exempt (SQL `NULLS DISTINCT`) — a primary key's NULL is
/// [`validate_existing_not_null`]'s, after the build. A repeated key tuple fails loud
/// `23505` naming `constraint_name`.
///
/// Equality is the **index's own**, through `constraints::index_key_identity` — the
/// same function `CREATE UNIQUE INDEX`'s build gate keys through. It used to be the
/// value tuple's structural (postcard) encoding, on the reasoning that every row of one
/// column shares a `Value` variant so equal values encode equally; that is true of a
/// *structural* equality and not of the index's, which is the key **encoding**'s and
/// therefore normalises what an order-preserving encoding cannot distinguish. Measured:
/// a `numeric` column holding `1.5` and `1.50` took `ADD CONSTRAINT UNIQUE` and
/// `ADD PRIMARY KEY` here and is `could not create unique index … Key (n)=(1.5) is
/// duplicated.` on 18.6 — after which pglet's own write path refused the row already
/// sitting in the table.
fn validate_existing_unique<S: Storage>(
    storage: &S,
    table: &Table,
    columns: &[String],
    constraint_name: &str,
) -> Result<()> {
    // Key column positions (columns are pre-validated to exist).
    let positions: Vec<usize> = columns
        .iter()
        .filter_map(|c| table.columns.iter().position(|tc| &tc.name == c))
        .collect();
    // Keyed by identity, valued by the **first** row's key tuple: that is the one 18.6's
    // DETAIL names (scan order, not the colliding row), and the two differ whenever the
    // index's equality calls two different spellings equal.
    let mut seen: HashMap<Vec<u8>, Vec<Value>> = HashMap::new();
    for (_, row) in storage.scan(&table.name)? {
        let key: Vec<Value> = positions
            .iter()
            .map(|&pos| row.get(pos).cloned().unwrap_or(Value::Null))
            .collect();
        // A NULL key is not a uniqueness candidate (NULLs distinct).
        if key.iter().any(Value::is_null) {
            continue;
        }
        match seen.entry(super::constraints::index_key_identity(&key)?) {
            std::collections::hash_map::Entry::Occupied(first) => {
                return Err(Error::DuplicateIndexKey {
                    index: constraint_name.to_string(),
                    detail: Some(Box::new(pglet_core::UniqueViolationDetail {
                        table: table.name.clone(),
                        key: Some(pglet_core::KeyDescription::new(
                            columns.to_vec(),
                            first
                                .get()
                                .iter()
                                .zip(&positions)
                                .map(|(v, &pos)| {
                                    table.columns.get(pos).map_or_else(
                                        || crate::types::value_to_text(v),
                                        |c| crate::types::value_to_text_with_oid(v, c.type_oid),
                                    )
                                })
                                .collect(),
                        )),
                    })),
                });
            }
            std::collections::hash_map::Entry::Vacant(slot) => {
                slot.insert(key);
            }
        }
    }
    Ok(())
}

/// Execute `ALTER TABLE … ALTER COLUMN … { SET DEFAULT expr | DROP DEFAULT }`.
///
/// Rewrites the column's catalog `default` in place (so omitted-column INSERTs keep
/// resolving it). A `nextval(seq)` default registers the sequence if absent, so the
/// pg_dump-restore path's re-attached SERIAL default resolves a real sequence.
///
/// **The target is resolved before the sequence is registered**, and that ordering is
/// a correctness property rather than tidiness: the autocommit path rolls back storage
/// on an error and leaves `self.schema` mutated, so registering first made
/// `ALTER TABLE t ALTER COLUMN <missing> SET DEFAULT nextval('ghost')` raise its
/// `42703` over a **durable phantom sequence** — one `nextval` then served and the
/// dump emitted a `CREATE SEQUENCE` for. The fixture harness cannot see that class at
/// all (every fixture runs inside `BEGIN … ROLLBACK`), so its guard is a unit test in
/// the autocommit bracket.
pub fn execute_alter_table_set_default(
    schema: &mut Schema,
    table_name: &str,
    column_name: &str,
    default: Option<&ColumnDefault>,
) -> Result<QueryResult> {
    // Resolve the target read-only first (see the doc comment).
    {
        let table = get_table_or_err(schema, table_name)?;
        let Some(column) = table.columns.iter().find(|c| c.name == column_name) else {
            return Err(crate::CatalogError::ColumnNotFound {
                table: table_name.to_string(),
                column: column_name.to_string(),
            }
            .into());
        };
        // An identity column's default is the engine's link to its own sequence, not
        // the user's to move or clear (`ATExecColumnDefault`, `42601` — with a hint
        // toward `DROP IDENTITY` on the `DROP DEFAULT` spelling). Both spellings arrive
        // here, `DROP DEFAULT` as `default: None`.
        //
        // Postgres stores no `pg_attrdef` row for an identity column at all, so the
        // refusal costs it nothing; pglet *does* model one, so accepting it severed the
        // identity/sequence link and the sequence — no longer the column's internal
        // implementation — then dropped through an ordinary `DROP SEQUENCE`, leaving the
        // default dangling. The `ALTER SEQUENCE … OWNED BY` gate is the other half of
        // the same fact (see `execute_alter_sequence`).
        //
        // Placed after the table and the column resolve and **before** the sequence
        // registration, which is where 18.6 puts it: a missing table is `42P01` and a
        // missing column `42703`, while `SET DEFAULT nextval('<no such sequence>')` on
        // an identity column is `42601` — the identity test outranks the name's
        // `regclass` resolution, and a fortiori pglet's own DDL-time `42809`
        // over-rejection, which stands in for a runtime error and runs last by rule.
        //
        // That is the order among the *executor's* gates, and it is not the whole
        // precedence: the parser refuses an unrecognized DEFAULT expression (`0A000` —
        // `nosuchfn()`) before this function is reached, where 18.6 answers `42601`,
        // and nothing placed here can outrank it — so a precedence claim about this gate
        // has to be quantified over what the front end accepts and rejects, not over
        // the gates alone (`docs/compatibility.md`'s Sequences row).
        if column.identity.is_some() {
            return Err(Error::IdentityColumnDefault {
                table: table_name.to_string(),
                column: column_name.to_string(),
            });
        }
        // The default's own verdict, behind the three gates above and ahead of the
        // sequence registration — measured on 18.6: a missing table is `42P01`, a
        // missing column `42703` and an identity column `42601`, each on a statement
        // whose default is `'x'` into an `INT`, while an ordinary column answers that
        // default's `22P02`.
        if let Some(default) = default {
            cook_column_default(default, column, DefaultTiming::Deferred, schema)?;
        }
    }
    // Register every referenced sequence (no-op if already present), before the
    // mutable table borrow below. A `SET DEFAULT nextval('x')` creates no ownership
    // edge — and a name a table or index holds is `42809`, not a second entry under
    // one relation name (see `ensure_sequence_relation`). Read through
    // `default_sequence_refs`, so an expression default (`nextval('x') + 1`) takes the
    // same kind gate and the same registration as the bare spelling; before that, it
    // took neither, and the column then answered `42P01` at every write.
    //
    // **Gate every name before registering any**, because a mid-loop refusal would
    // otherwise leave the earlier sequences of the same statement registered — the
    // durable-phantom shape the autocommit path cannot undo.
    let default_sequences = default.map(default_sequence_refs).unwrap_or_default();
    for seq in &default_sequences {
        ensure_sequence_relation(schema, seq)?;
    }
    for seq in default_sequences {
        schema
            .sequences
            .entry(seq)
            .or_insert_with(|| Sequence::new(1));
    }
    if let Some(col) = schema
        .get_table_mut(table_name)
        .and_then(|t| t.columns.iter_mut().find(|c| c.name == column_name))
    {
        col.default = default.cloned();
    }
    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// Execute `ALTER TABLE … ALTER COLUMN … SET NOT NULL`.
///
/// Enforces NOT NULL on an existing column: every existing row must already hold a
/// non-NULL value (else `23502`), then the column's `nullable` flag is flipped so
/// later NULL inserts are rejected — the same shape as `ADD CONSTRAINT … PRIMARY
/// KEY` setting its key columns NOT NULL. Validate-before-mutate: the flag is only
/// flipped once the scan confirms no offending row, so a rejection leaves the
/// catalog untouched.
pub fn execute_alter_table_set_not_null<S: Storage>(
    schema: &mut Schema,
    storage: &S,
    table_name: &str,
    column_name: &str,
) -> Result<QueryResult> {
    let pos = {
        let table = get_table_or_err(schema, table_name)?;
        table
            .columns
            .iter()
            .position(|c| c.name == column_name)
            .ok_or_else(|| crate::CatalogError::ColumnNotFound {
                table: table_name.to_string(),
                column: column_name.to_string(),
            })?
    };
    // Reject if any existing row already holds a NULL (a missing cell counts as
    // NULL) — Postgres raises `23502` here.
    for (_, row) in storage.scan(table_name)? {
        match row.get(pos) {
            Some(value) if !value.is_null() => {}
            _ => {
                return Err(Error::NotNullColumnHasNulls {
                    table: table_name.to_string(),
                    column: column_name.to_string(),
                });
            }
        }
    }
    if let Some(table) = schema.get_table_mut(table_name)
        && let Some(col) = table.columns.iter_mut().find(|c| c.name == column_name)
    {
        col.nullable = false;
    }
    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// Execute `ALTER TABLE … ALTER COLUMN col ADD GENERATED { ALWAYS | BY DEFAULT } AS
/// IDENTITY [( options )]` — make an existing column an identity column.
///
/// This is the form pg_dump emits (its restore re-attaches the identity after the
/// data load, and emits **no** separate `CREATE SEQUENCE` for it). It resolves and
/// registers the implicit sequence — the declared `SEQUENCE NAME <n>` pg_dump carries,
/// or a minted `<table>_<col>_seq` — then sets the column's `identity` and its
/// `NextVal` default, the shape an inline `CREATE TABLE (… GENERATED …)` builds.
///
/// **The column must already be declared `NOT NULL`** (`55000`, 18.6's
/// `ATExecAddIdentity`). An identity column is `NOT NULL`, and this statement reads no
/// row, so it may not be what first says so: setting the flag here would declare
/// `NOT NULL` over whatever the column holds, a NULL among it. The refusal reads the
/// declaration alone — it is the same over an empty table and over rows holding no NULL
/// — which leaves `SET NOT NULL`, the statement that does read the rows, as the way in.
///
/// **And it must have no default** (`55000`, the same function's last check): a
/// constant's, an expression's or a `SERIAL`'s. Writing the identity's `NextVal` over one
/// would drop it with no word, and over a `SERIAL` restart the column's ids at the new
/// sequence's `START WITH`. `DROP DEFAULT` first is the way in. An identity column's own
/// `NextVal` is not asked: the already-identity refusal answers for that column, earlier.
///
/// The checks run ahead of the first catalog write, the sequence's registration, so a
/// refused statement leaves no sequence behind. The `NOT NULL` and the default refusals
/// stand where 18.6 raises them, the declaration's ahead of the default's: 18.6 creates
/// the sequence before `ATExecAddIdentity` looks at the column, so a non-integer type
/// (`22023`) and a taken sequence name (`42P07`, or `42710` for a type's) each outrank
/// both, measured. The already-identity refusal does not: it is ahead of the sequence's
/// name here and behind it on 18.6 (`docs/ddl.md` § The mirror). Nor are the sequence's
/// options judged where 18.6 judges them, ahead of both refusals: they are not judged
/// here (`EDGES.md` rows 215 and 315).
///
/// # Errors
///
/// `CatalogError::ColumnNotFound` (`42703`), `Error::AlreadyIdentityColumn`
/// (`55000`), a non-integer column type (`22023`),
/// `CatalogError::SequenceAlreadyExists` (`42P07`) for a taken declared name,
/// `Error::RelationTypeAlreadyExists` (`42710`) for a sequence name a type holds,
/// `Error::IdentityColumnNotDeclaredNotNull` (`55000`), or
/// `Error::IdentityColumnHasDefault` (`55000`).
pub fn execute_alter_table_add_identity(
    schema: &mut Schema,
    table_name: &str,
    column_name: &str,
    generation: IdentityGeneration,
    start: i64,
    sequence_name: Option<&str>,
) -> Result<QueryResult> {
    // Validate the column exists, isn't already an identity column (`55000`), and is
    // integer-typed (`22023`) before mutating anything.
    let (declared_not_null, has_default) = {
        let table = get_table_or_err(schema, table_name)?;
        let col = table
            .columns
            .iter()
            .find(|c| c.name == column_name)
            .ok_or_else(|| crate::CatalogError::ColumnNotFound {
                table: table_name.to_string(),
                column: column_name.to_string(),
            })?;
        if col.identity.is_some() {
            return Err(Error::AlreadyIdentityColumn {
                table: table_name.to_string(),
                column: column_name.to_string(),
            });
        }
        ensure_identity_integer(col.type_oid)?;
        (!col.nullable, col.default.is_some())
    };

    // Resolve the implicit sequence's name against the live relation namespace, before
    // the mutable table borrow. Resolved, not formatted: an engine-chosen name
    // disambiguates rather than being silently shared (measured on 18.6: with
    // `ag_x_seq` taken, `ALTER TABLE ag ALTER COLUMN x ADD GENERATED BY DEFAULT AS
    // IDENTITY` binds the column to `ag_x_seq1`), while a declared `SEQUENCE NAME` is
    // taken verbatim and a taken one is `42P07`.
    let seq = resolve_implicit_sequence_name(
        schema,
        &HashSet::new(),
        table_name,
        column_name,
        sequence_name,
    )?;

    // The last checks, and the last step ahead of a catalog write: the column's own
    // declaration, then its default. Behind the sequence's refusals and in this order,
    // as on 18.6.
    if !declared_not_null {
        return Err(Error::IdentityColumnNotDeclaredNotNull {
            table: table_name.to_string(),
            column: column_name.to_string(),
        });
    }
    if has_default {
        return Err(Error::IdentityColumnHasDefault {
            table: table_name.to_string(),
            column: column_name.to_string(),
        });
    }

    schema
        .sequences
        .entry(seq.clone())
        .or_insert_with(|| Sequence::owned_by(start, table_name, column_name));

    let table =
        schema
            .get_table_mut(table_name)
            .ok_or_else(|| crate::CatalogError::TableNotFound {
                table: table_name.to_string(),
            })?;
    if let Some(col) = table.columns.iter_mut().find(|c| c.name == column_name) {
        col.identity = Some(generation);
        col.default = Some(ColumnDefault::NextVal(seq));
    }
    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// Execute `CREATE SEQUENCE [IF NOT EXISTS] name [options]`.
///
/// The name is checked against the whole **relation namespace** — a sequence is a
/// `pg_class` relation, so it collides with a table and an index too (`42P07`,
/// measured in every polarity on 18.6). This used to be an `or_insert`, which made a
/// re-`CREATE` silently discard the new `START WITH` and keep the old counter.
///
/// The checks run in Postgres's order, which is observable:
/// `heap_create_with_catalog` raises `42P07` **before** `process_owned_by` resolves
/// the owner, so `CREATE SEQUENCE <existing> OWNED BY t.nosuch` is `42P07`, not
/// `42703`; and the `IF NOT EXISTS` skip (`DefineSequence`) precedes both, so the
/// same statement with `IF NOT EXISTS` is a plain NOTICE — the owner is never
/// resolved at all. Everything fallible is therefore ahead of the one mutation,
/// which the autocommit path also requires (a phantom sequence would be durable).
///
/// A `RESTART` clause is admitted here (`SeqOptElem` carries it on `CREATE` too) and
/// moves only the **counter**: `init_params` writes `last_value` and leaves
/// `seqform->seqstart` at the `START WITH`, so `CREATE SEQUENCE s START WITH 10
/// RESTART WITH 5` hands out `5` and a later bare `ALTER SEQUENCE s RESTART` returns
/// to `10` (measured on 18.6). Dropping the clause — which is what sharing the option
/// parser with `ALTER` without reading the field did — turned a loud `42601` into a
/// silently wrong first value.
///
/// # Errors
///
/// `CatalogError::SequenceAlreadyExists` (`42P07`), or an owner-resolution error
/// from [`resolve_sequence_owner`] (`42P01` / `42703` / `42809`).
pub fn execute_create_sequence(
    schema: &mut Schema,
    name: &str,
    start: i64,
    restart: Option<SequenceRestart>,
    if_not_exists: bool,
    owned_by: Option<&SequenceOwnerSpec>,
) -> Result<QueryResult> {
    if schema.relation_name_taken(name) {
        if if_not_exists {
            return Ok(QueryResult::ddl_with_notice(
                "CREATE SEQUENCE",
                format!("relation \"{name}\" already exists, skipping"),
            ));
        }
        return Err(crate::CatalogError::SequenceAlreadyExists {
            sequence: name.to_string(),
        }
        .into());
    }
    // A sequence has no row type, but `heap_create_with_catalog` tests the type
    // namespace for it all the same, after the relation name and ahead of the owner —
    // and past the `IF NOT EXISTS` skip, which reads the relation namespace only.
    if schema.type_name_taken(name) {
        return Err(Error::RelationTypeAlreadyExists(name.to_string()));
    }
    let owner = owned_by
        .map(|spec| resolve_sequence_owner(schema, spec))
        .transpose()?
        .flatten();
    let mut sequence = Sequence {
        owner,
        ..Sequence::new(start)
    };
    // A bare `RESTART` is `start` — which the seeding above already produced — so
    // only the `RESTART WITH n` form is observable here.
    if let Some(SequenceRestart::To(n)) = restart {
        sequence.last_value = n.saturating_sub(1);
    }
    schema.sequences.insert(name.to_string(), sequence);
    Ok(QueryResult::ddl("CREATE SEQUENCE"))
}

/// Execute `ALTER SEQUENCE [IF EXISTS] name [options]`.
///
/// Two option tails carry meaning; the rest of the tail was consumed by the parser
/// and has no effect. `START [WITH] n` moves the recorded `START WITH` and
/// `RESTART [[WITH] n]` moves the counter — `RESTART` bare returns it to `start`, so
/// the two interact, and `START WITH` is applied **first** regardless of written
/// order (`START WITH 42 RESTART` and `RESTART START WITH 42` both leave the next
/// `nextval` at 42 on 18.6). `OWNED BY` sets or, with `NONE`, clears the owner.
///
/// Two measured details a plausible implementation gets wrong. A `RESTART` does
/// **not** disturb `currval`, which is per-session state and lives outside the
/// catalog — so nothing here touches [`super::sequence::SequenceSession`]. And the
/// `RESTART` **is** transactional: `Plan::swapped_sequences` records the cell it
/// replaces, so a `ROLLBACK` puts the counter back.
///
/// The target checks run in `sequence.c`'s order, and it is not the obvious one:
/// `validate_relation_kind` runs **before** the `IF EXISTS` skip, so a table or
/// index named here is `42809` even under `IF EXISTS`, while a name no relation
/// holds is `42P01` (or a NOTICE). Both outrank the owner resolution.
///
/// **An identity column's sequence refuses every `OWNED BY`.** `process_owned_by`
/// tests `sequenceIsOwned(seq, DEPENDENCY_INTERNAL, …)` and raises `0A000` — for
/// `OWNED BY NONE`, for another table's column, for a plain sibling column, and for
/// the identity column *itself* (all measured on 18.6). The refusal is load-bearing
/// rather than pedantic: pglet models the identity link as the column's `NextVal`
/// default plus the sequence's `owner`, and detaching the edge lets an ordinary
/// `DROP SEQUENCE` (or a `DROP TABLE` / `DROP COLUMN` of whatever the edge was moved
/// to) remove the sequence while the default still names it — every write then
/// `42P01`, and the dump emits `DEFAULT nextval(…)` with no `CREATE SEQUENCE` for it.
///
/// It runs **after** [`resolve_sequence_owner`], because `process_owned_by` opens the
/// named relation, checks its kind and looks up the column before reaching the
/// identity test: a missing owner table (`42P01`), a missing owner column (`42703`)
/// and a wrong-kind owner (`42809`) each outrank it, all measured. And before any
/// field is written, so a refused statement leaves the counter and the `start` alone
/// (`ALTER SEQUENCE s RESTART WITH 50 OWNED BY NONE` moves nothing on either engine).
///
/// # Errors
///
/// `Error::WrongRelationKind` (`42809`), `Error::SequenceNotFound` (`42P01`), an
/// owner-resolution error (`42809` / `42P01` / `42703`), or
/// `Error::IdentitySequenceOwnershipChange` (`0A000`).
pub fn execute_alter_sequence(
    schema: &mut Schema,
    name: &str,
    if_exists: bool,
    start: Option<i64>,
    restart: Option<SequenceRestart>,
    owned_by: Option<&SequenceOwnerSpec>,
) -> Result<QueryResult> {
    ensure_sequence_relation(schema, name)?;
    if !schema.sequences.contains_key(name) {
        if if_exists {
            return Ok(QueryResult::ddl_with_notice(
                "ALTER SEQUENCE",
                format!("relation \"{name}\" does not exist, skipping"),
            ));
        }
        return Err(Error::SequenceNotFound(name.to_string()));
    }
    // Resolve the owner change against the (immutably borrowed) catalog before taking
    // the mutable slot — and before any field is written, so a bad owner leaves the
    // sequence untouched.
    let owner_change = owned_by
        .map(|spec| resolve_sequence_owner(schema, spec))
        .transpose()?;
    if owner_change.is_some()
        && let Some(identity) = identity_sequence_owner(schema, name)
    {
        return Err(Error::IdentitySequenceOwnershipChange {
            sequence: name.to_string(),
            table: identity.table,
        });
    }
    let Some(sequence) = schema.sequences.get_mut(name) else {
        // Unreachable (the `contains_key` above), but loud rather than a panic.
        return Err(Error::SequenceNotFound(name.to_string()));
    };
    if let Some(new_start) = start {
        sequence.start = new_start;
    }
    if let Some(restart_to) = restart {
        let next = match restart_to {
            SequenceRestart::ToStart => sequence.start,
            SequenceRestart::To(n) => n,
        };
        sequence.last_value = next.saturating_sub(1);
    }
    if let Some(owner) = owner_change {
        sequence.owner = owner;
    }
    Ok(QueryResult::ddl("ALTER SEQUENCE"))
}

/// Execute `DROP SEQUENCE [IF EXISTS] name [CASCADE | RESTRICT]`.
///
/// The target checks run in `RemoveRelations`' order, which is the same one
/// `DROP TABLE` and `DROP INDEX` already follow: the **kind** is decided on a name
/// that *resolves*, so only a name no relation holds at all reaches
/// `DropErrorMsgNonExistent` and its `IF EXISTS` skip. `DROP SEQUENCE [IF EXISTS]
/// <a table>` and `… <an index>` are therefore `42809` either way (measured on 18.6),
/// never a silent success that leaves the next statement insisting the name is taken.
///
/// Two dependency edges then decide the drop, and they are *not* the same rule wearing
/// two hats (both measured on 18.6):
///
/// - An **identity** column's sequence is its INTERNAL implementation: `2BP01`, with
///   `CASCADE` and with `IF EXISTS` alike. The column has to go instead.
/// - A plain column `DEFAULT nextval('s')` — which is what a `SERIAL` carries — is a
///   NORMAL `pg_attrdef` edge: `2BP01` unless `CASCADE`, which sheds **that column's
///   default** and leaves the column, its `NOT NULL` and its rows. Where both are
///   present the INTERNAL one wins.
///
/// An owned sequence whose column carries no default has neither edge and drops
/// cleanly, the owner going with it.
///
/// # Errors
///
/// [`Error::WrongObjectType`] (`42809`), [`Error::SequenceNotFound`] (`42P01`), or a
/// `2BP01` from [`Dependents::ensure_droppable`].
pub fn execute_drop_sequence(
    schema: &mut Schema,
    names: &[String],
    if_exists: bool,
    cascade: bool,
) -> Result<QueryResult> {
    // The same three phases `execute_drop_table` documents, and phase 1 is the same
    // gate: every name resolved and kind-checked positionally before anything else,
    // so `DROP SEQUENCE <an identity column's sequence>, <a missing name>` is `42P01`
    // and not the `2BP01` the first name earns on its own (measured on 18.6).
    let (targets, notices) =
        resolve_drop_targets(schema, names, RelationKind::Sequence, if_exists)?;

    // Everything fallible before the first catalog write: on the autocommit path a
    // half-applied DDL is durable (see `execute_create_table`).
    //
    // The in-set filter is applied for uniformity and is **inert** here: a sequence's
    // blocking dependents are all owned by *tables*, and a table cannot share a name
    // with a sequence (`Schema::relation_name_taken`), so no entry can match a set of
    // sequence names.
    let set: HashSet<&str> = targets.iter().copied().collect();
    let mut resolved: Vec<Dependents> = Vec::with_capacity(targets.len());
    for name in &targets {
        let target = DropTarget::Sequence { sequence: name };
        let mut dependents = Dependents::resolve(schema, target);
        dependents.retain_dependents_outside(&set);
        dependents.ensure_droppable(target, cascade)?;
        resolved.push(dependents);
    }

    // Under CASCADE this clears the dependent columns' defaults; `auto_sequences` is
    // empty for a sequence target, so the sequences themselves are removed here.
    Dependents::drop_all_from_catalog(schema, &resolved, cascade);
    for name in &targets {
        schema.sequences.remove(*name);
    }
    Ok(QueryResult::ddl_with_notices("DROP SEQUENCE", notices))
}

/// Reject a statement whose sequence *operand* is a relation of another kind
/// (`42809`), leaving "not a relation at all" to the caller.
///
/// Shared by `ALTER SEQUENCE`'s target, by `nextval`/`currval`/`setval`, and by the
/// DDL-time registration of a user's `DEFAULT nextval('x')` — all of which are
/// Postgres's one `validate_relation_kind` (`cannot open relation "x"`), reached
/// from `init_sequence`. The **owner** side of an `OWNED BY` is *not* this error;
/// `process_owned_by` words its own (see [`Error::SequenceOwnerWrongKind`]).
///
/// The DDL-registration caller is a **deliberate divergence**, and the reason it is
/// deliberate: Postgres resolves `'x'::regclass` at parse analysis and checks the
/// *kind* only when the sequence is first read, so `CREATE TABLE q2 (b INT DEFAULT
/// nextval('q1'))` over a table `q1` is accepted there and every `INSERT` is `42809`
/// (measured on 18.6 for `CREATE TABLE`, `ADD COLUMN` and `SET DEFAULT` alike).
/// pglet has no `regclass` and *auto-creates* the named sequence at DDL time, so
/// deferring the check would not defer the error — it would put one name in
/// `tables` and `sequences` at once, the state [`Schema::relation_name_taken`]
/// declares impossible, and the database would then dump to a script carrying both a
/// `CREATE SEQUENCE q1` and a `CREATE TABLE q1` that neither engine reloads.
/// Rejecting the DDL is stricter than Postgres, under the same SQLSTATE and message.
/// The over-rejection is narrow but not empty: Postgres rejects every write that
/// *evaluates* the default (so an `ADD COLUMN` on a populated table is `42809` there
/// too, at the rewrite), while a write naming the column explicitly never touches it and
/// succeeds. What the DDL callers must therefore not do is let this error outrank one
/// Postgres raises at DDL time — it stands in for a *runtime* failure, so each caller
/// runs it **after** every other check (see `execute_create_table` and
/// `execute_alter_table_add_column`).
///
/// It reads the **catalog**, so on the one path where the relation namespace is not
/// yet the catalog — a `CREATE TABLE` naming its own table — it is blind and
/// [`ensure_sequence_relation_for_new_table`] is the gate. The other callers
/// (`ALTER SEQUENCE`, the sequence functions, `ADD COLUMN`, `SET DEFAULT`) resolve
/// against a committed catalog, which suffices *for the kind check*. It does not
/// suffice for the name's **claim**: the sequence a `CREATE TABLE` or an `ADD COLUMN`
/// is about to register is not in the catalog either, and a constraint index of the
/// same statement has to see it — that seeding is the callers' own, not this gate's.
///
/// # Errors
///
/// `Error::WrongRelationKind`.
pub fn ensure_sequence_relation(schema: &Schema, name: &str) -> Result<()> {
    match schema.relation_kind(name) {
        Some(RelationKind::Sequence) | None => Ok(()),
        Some(kind) => Err(Error::WrongRelationKind {
            relation: name.to_string(),
            kind,
        }),
    }
}

/// [`ensure_sequence_relation`] from **inside** the `CREATE TABLE` that is building
/// `creating` — the one caller whose relation namespace is not the catalog yet.
///
/// A gate that reads the catalog cannot see the table the statement is halfway
/// through creating, so `CREATE TABLE e3 (b INT DEFAULT nextval('e3'), c INT)` slipped
/// past the shared gate and put `e3` in `tables` and `sequences` at once — after which
/// `nextval('e3')` answered `42809` (`relation_kind` reads `tables` first) while an
/// `INSERT … VALUES (DEFAULT)` resolved the sequence and returned 1, and the database's
/// dump carried both a `CREATE SEQUENCE e3` and a `CREATE TABLE e3`, of which neither
/// stock Postgres nor pglet's own restore loads the second.
///
/// `42809` and not `42P07`, deliberately: 18.6 *accepts* that `CREATE TABLE` — the
/// default is cooked once the heap exists, so `'e3'::regclass` binds the **table** —
/// and raises `42809 cannot open relation "e3"` at the first write. So this is the
/// same documented DDL-time over-rejection [`ensure_sequence_relation`] describes,
/// spelled with the SQLSTATE Postgres eventually reaches, rather than a second one.
///
/// # Errors
///
/// `Error::WrongRelationKind`.
fn ensure_sequence_relation_for_new_table(
    schema: &Schema,
    creating: &str,
    name: &str,
) -> Result<()> {
    if name == creating {
        return Err(Error::WrongRelationKind {
            relation: name.to_string(),
            kind: RelationKind::Table,
        });
    }
    ensure_sequence_relation(schema, name)
}

/// Resolve an `OWNED BY` clause against the catalog: `NONE` detaches (`Ok(None)`),
/// `<table>.<column>` must name a live column of a **table**.
///
/// The kind check is the owner's own, not the target's: `ALTER SEQUENCE s OWNED BY
/// <an index>.x` is `42809 sequence cannot be owned by relation "…"` on 18.6, with
/// the same `42809` for a sequence — a *different sentence* from the target path's
/// `cannot open relation`, so this cannot reuse [`ensure_sequence_relation`]. It
/// runs ahead of the column lookup because `process_owned_by` opens the relation
/// (and validates its kind) before touching `pg_attribute`.
///
/// # Errors
///
/// `Error::SequenceOwnerWrongKind` (`42809`), `CatalogError::TableNotFound`
/// (`42P01`), or `CatalogError::ColumnNotFound` (`42703`) — measured in that order.
fn resolve_sequence_owner(
    schema: &Schema,
    spec: &SequenceOwnerSpec,
) -> Result<Option<SequenceOwner>> {
    let SequenceOwnerSpec::Column { table, column } = spec else {
        return Ok(None);
    };
    if let Some(kind @ (RelationKind::Index | RelationKind::Sequence)) = schema.relation_kind(table)
    {
        return Err(Error::SequenceOwnerWrongKind {
            relation: table.clone(),
            kind,
        });
    }
    let owner_table = get_table_or_err(schema, table)?;
    if !owner_table.columns.iter().any(|c| &c.name == column) {
        return Err(crate::CatalogError::ColumnNotFound {
            table: table.clone(),
            column: column.clone(),
        }
        .into());
    }
    Ok(Some(SequenceOwner {
        table: table.clone(),
        column: column.clone(),
    }))
}

/// The name an **implicit** sequence takes — the one behind a `SERIAL` or identity
/// column — when the engine chooses it, disambiguated the way Postgres's
/// `ChooseRelationName` does.
///
/// The engine chose this name, so a collision is *disambiguated* rather than refused:
/// `t_a_seq`, `t_a_seq1`, `t_a_seq2`, … ([`choose_relation_name`], which also cuts the
/// name to 18.6's 63 bytes). The same
/// declared/auto asymmetry [`ConstraintNames`] implements for constraints, over a
/// different namespace — and the difference matters in both directions:
///
/// - a sequence must clear the whole **relation** namespace, so a *table* or an
///   *index* named `t_a_seq` pushes the column to `t_a_seq1` exactly as another
///   sequence does;
/// - it must **not** consult `pg_constraint`, because `transformColumnDefinition`
///   calls `ChooseRelationName(…, isconstraint = false)`. Measured on 18.6: a CHECK
///   constraint named `c2_b_seq` leaves `CREATE TABLE c2 (b SERIAL)` on `c2_b_seq`.
///
/// It reads the **committed catalog only**, never the names this same statement has
/// already claimed — the one place this differs from `ConstraintNames`' `catalog ∪
/// claimed`, and it is measured rather than assumed. `ChooseRelationName` runs during
/// `transformCreateStmt`, which merely *appends* a `CreateSeqStmt` to the before-list;
/// nothing exists yet for the next column's lookup to see. So `CREATE TABLE zf (b INT
/// GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME zf_a_seq), a SERIAL)` is `42P07` on
/// 18.6 — the SERIAL picks the already-spoken-for `zf_a_seq` and the second
/// `CREATE SEQUENCE` fails — where consulting the claimed set would silently hand it
/// `zf_a_seq1`. [`resolve_implicit_sequence_name`] is where the claimed set is read,
/// as a *rejection*.
fn mint_sequence_name(schema: &Schema, table: &str, column: &str) -> String {
    choose_relation_name(schema, table, column, "seq")
}

/// `<name1>_<name2>_<label>` as 18.6 chooses it: cut to 63 bytes by
/// [`make_object_name`], and where `taken` holds the candidate, a digit moved onto the
/// **label** (`key`, `key1`, `key2`, …) with the name cut again around it — the loop
/// `ChooseRelationName` and `ChooseConstraintName` share, each over its own namespace.
/// So a disambiguated name is no longer than an undisambiguated one.
fn choose_name(name1: &str, name2: &str, label: &str, taken: impl Fn(&str) -> bool) -> String {
    let mut modified = label.to_string();
    let mut pass = 0_u32;
    loop {
        let candidate = make_object_name(name1, name2, &modified);
        if !taken(&candidate) {
            return candidate;
        }
        pass += 1;
        modified = format!("{label}{pass}");
    }
}

/// A name the engine chooses for a relation that is not a constraint's — 18.6's
/// `ChooseRelationName(…, isconstraint = false)`, behind an implicit sequence
/// ([`mint_sequence_name`]) and an unnamed `CREATE INDEX` ([`choose_index_name`]).
///
/// What [`choose_name`] steps around here is the relation
/// namespace ([`Schema::relation_name_taken`] — tables, indexes and sequences), and
/// not a constraint's name or a type's.
fn choose_relation_name(schema: &Schema, name1: &str, name2: &str, label: &str) -> String {
    choose_name(name1, name2, label, |candidate| {
        schema.relation_name_taken(candidate)
    })
}

/// The name an unnamed `CREATE INDEX ON <table> (…)` takes — 18.6's `ChooseIndexName`
/// for an index that is no constraint's: the table's name, the key parts' names
/// joined by `_`, and the label `idx` — for a `UNIQUE` index too, `key` being a
/// constraint's label.
///
/// A key part's name is 18.6's `ChooseIndexColumnNames`: a bare column's own name, an
/// expression's `FigureIndexColname` — the name a `SELECT` would give that expression
/// as an output column ([`Expr::output_name`]: `lower(a)` is `lower`, `(a)::text` is
/// `a`), with `expr` where a `SELECT` says `?column?` — and a name an earlier part of
/// the same key already took gets a digit of its own (`a, a` is `a_a1`; `expr`,
/// `expr1`).
fn choose_index_name(schema: &Schema, table: &str, key: &IndexKey) -> String {
    let mut names: Vec<String> = Vec::with_capacity(key.len());
    match key {
        IndexKey::Columns(columns) => {
            for column in columns {
                push_key_part_name(&mut names, column);
            }
        }
        IndexKey::Exprs(parts) => {
            for part in parts {
                let name = part.output_name();
                let nameless = name == "?column?" && !matches!(part, Expr::Column { .. });
                push_key_part_name(&mut names, if nameless { "expr" } else { &name });
            }
        }
    }
    choose_relation_name(schema, table, &names.join("_"), "idx")
}

/// Add one key part's name to `names`, with a digit of its own where an earlier part
/// already took it.
///
/// 18.6 also cuts the digited name to 63 bytes, and that cut is not reproduced. Over
/// names 18.6 can hold — 63 bytes or fewer — a part needs it only when it is within its
/// digits' length of 63 bytes, and the earlier part it repeats is then already more of
/// the joined name than [`make_object_name`] keeps: the digit is past the index name's
/// cut with or without it (`ddl/create_index_unnamed_repeated_long_key_name.sql` measures
/// a 63-byte column).
fn push_key_part_name(names: &mut Vec<String>, original: &str) {
    let mut name = original.to_string();
    let mut pass = 0_u32;
    while names.contains(&name) {
        pass += 1;
        name = format!("{original}{pass}");
    }
    names.push(name);
}

/// The name an implicit sequence takes, **declared or minted**, gated against the
/// relation namespace and the names this statement has already spoken for.
///
/// The declared/auto asymmetry, at the one gate every implicit-sequence site goes
/// through (`CREATE TABLE`, `ADD COLUMN`, `ALTER COLUMN … ADD GENERATED`): a
/// `SEQUENCE NAME <n>` is the **user's**, so it is taken verbatim and refused
/// (`42P07`, `relation "<n>" already exists` — measured for a colliding table, index
/// and sequence alike) rather than disambiguated; an absent one is the **engine's**
/// and disambiguates through [`mint_sequence_name`]. `SERIAL` has no spelling for a
/// declared name, so it is always the second branch.
///
/// `claimed` rejects on both branches, because two `CreateSeqStmt`s in one statement
/// are two `CREATE SEQUENCE`s and the second sees the first (`CommandCounterIncrement`
/// — the same reason `ConstraintNames` carries a claimed set).
///
/// A `CREATE TABLE`'s **own table name** is deliberately *not* rejected here, and that
/// is not an oversight: the sequence is created first and the collision surfaces when
/// the heap is, so 18.6 reports it against the table (`42P07 relation "a1" already
/// exists`) and a *later* column's collision with an already-taken name outranks it.
/// `execute_create_table` therefore tests the in-flight set at the table's own `42P07`
/// instead. On the `ALTER` routes the table is already in the catalog, so
/// `relation_name_taken` covers it here (measured: `ADD COLUMN … (SEQUENCE NAME <the
/// table>)` and `ADD GENERATED … (SEQUENCE NAME <the table>)` are both `42P07`).
///
/// # Errors
///
/// `CatalogError::SequenceAlreadyExists` (`42P07`).
fn resolve_implicit_sequence_name(
    schema: &Schema,
    claimed: &HashSet<String>,
    table: &str,
    column: &str,
    declared: Option<&str>,
) -> Result<String> {
    let name = match declared {
        Some(declared) if schema.relation_name_taken(declared) => {
            return Err(crate::CatalogError::SequenceAlreadyExists {
                sequence: declared.to_string(),
            }
            .into());
        }
        Some(declared) => declared.to_string(),
        None => mint_sequence_name(schema, table, column),
    };
    if claimed.contains(&name) {
        return Err(crate::CatalogError::SequenceAlreadyExists { sequence: name }.into());
    }
    // The type namespace, behind the relation one, as `heap_create_with_catalog` asks
    // it of a sequence too — and of the **minted** name as well as a declared one,
    // because `ChooseRelationName` steps around `pg_class` only: with an enum
    // `h_a_seq`, 18.6 answers `CREATE TABLE h (a SERIAL)` with `42710` rather than
    // picking `h_a_seq1` (measured).
    if schema.type_name_taken(&name) {
        return Err(Error::RelationTypeAlreadyExists(name));
    }
    Ok(name)
}

/// Execute CREATE INDEX.
///
/// Builds the index data in storage, **then** inserts it into the catalog:
/// `storage.create_index` back-fills entries from the existing rows and maintains the
/// index on every subsequent write, and the two catalog writes — the OID counter and
/// `schema.create_index` — follow it, because the build is the statement's last fallible
/// step and the autocommit path cannot undo a catalog write (`docs/ddl.md` § A failed
/// index build). The index is then probeable via `storage.index_lookup`.
///
/// Composite indexes are built, maintained, and enforced (the full multi-column
/// key is probed at write time for uniqueness); the read-side equality probe,
/// however, only consults single-column indexes — a composite index doesn't yet
/// accelerate a `WHERE` point read.
///
/// The planner consults single-column indexes for equality point reads: a
/// `WHERE indexed_col = <value>` lowers to `Plan::IndexScan` (a probe) instead of
/// `Filter(Scan)`. CREATE INDEX builds and maintains the index for those probes
/// and for the write-path probes that PK/UNIQUE enforcement uses (`enforced`
/// records whether this unique index participates in that enforcement).
// `unique`, `enforced`, and `nulls_not_distinct` are independent catalog facts
// (every `unique` index is enforced — plain or expression — and NND is
// orthogonal), so they stay distinct params; this keeps the positional dispatch
// uniform with the sibling `execute_*` functions rather than introducing a
// one-off options struct.
#[allow(clippy::too_many_arguments, clippy::fn_params_excessive_bools)]
pub fn execute_create_index<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    name: Option<&str>,
    table: &str,
    key: IndexKey,
    unique: bool,
    enforced: bool,
    predicate: Option<Expr>,
    nulls_not_distinct: bool,
    method: IndexMethod,
    if_not_exists: bool,
) -> Result<QueryResult> {
    // The **target relation** is resolved first, and its kind checked, because that is
    // where `DefineIndex` puts both: `table_open` then the `relkind` switch
    // (`indexcmds.c`, `cannot create index on relation "s"` → `42809`), well ahead of
    // the name conflict, which `index_create` raises last. So a missing table
    // (`42P01`) and a wrong-kind one (`42809`) each beat a taken index name *and* the
    // `IF NOT EXISTS` skip — all four arrangements measured on 18.6. Testing the name
    // first is what made `CREATE INDEX IF NOT EXISTS <a taken name> ON <a sequence>`
    // report success.
    let table_obj = get_table_or_err(schema, table)?;

    // The stored expressions come **first**, because 18.6 analyses the whole
    // `IndexStmt` — key expressions and `WHERE` alike — in `transformIndexStmt`,
    // before `DefineIndex` resolves a bare key column or an opclass. Measured on 18.6:
    // `CREATE INDEX z ON t (nosuchcol) WHERE k` is `42804`, and
    // `… USING gin (intcol) WHERE txt LIKE 1` is `42883` rather than the GIN method's
    // refusal. Moving this call above `validate_columns_exist` and
    // `validate_index_method` put pglet on both, from the later gate's `42703` and
    // `0A000` respectively.
    //
    // A **mixed key** is a live residual the move did not touch:
    // `… (nosuchcol, (txt LIKE 1))` is `42883` on 18.6 and
    // `42703 column "nosuchcol" does not exist` here, before the move and after. A
    // mixed key folds to `IndexKey::Exprs` (`parser::ddl`), and `index_stored_exprs`
    // hands `validate_stored_expr` every part of one — the bare-column parts
    // included — where `transformIndexStmt` transforms only an `IndexElem` that
    // carries an `expr` and leaves a bare name to `DefineIndex`. Narrowing that
    // enumeration would move a second gate's position, which is its own decision and
    // not this one's.
    validate_index_exprs(&key, predicate.as_ref(), &table_obj.columns, &schema.enums)?;

    // 18.6's volatility gate, where `DefineIndex` raises it: the predicate
    // (`CheckPredicate`) and then each key expression (`ComputeIndexAttrs`), after parse
    // analysis and ahead of a bare key column's `42703`, the index name and the
    // `IF NOT EXISTS` skip — all measured.
    refuse_mutable_index_functions(&key, predicate.as_ref(), schema, table)?;

    // Validate each key column NAME exists in the table's row order (its position
    // is resolved by `index_spec_from_catalog` below). Only a PLAIN (bare-column)
    // index has column names to validate; an expression index keys via evaluated
    // `Expr`s (column refs resolved at eval time), so it skips this check.
    if let Some(cols) = key.bare_columns() {
        validate_columns_exist(table_obj, cols)?;
    }

    // The **GIN** half of the method check — the half Postgres raises ahead of the
    // name (`amcanunique`, `ResolveOpClass`). Its B-tree half is pglet-only and runs
    // after the name conflict below; see `validate_btree_key_type`.
    validate_index_method(method, unique, &key, table_obj)?;

    // The name conflict is tested **last** (`index_create`, after `ComputeIndexAttrs`),
    // so a bad key column outranks it and outranks the `IF NOT EXISTS` skip:
    // `CREATE INDEX IF NOT EXISTS <taken> ON t (nosuchcol)` is `42703` on both engines.
    // The whole relation namespace counts, not just the indexes: `CREATE INDEX t ON …`
    // where `t` is a table is `42P07` in Postgres, since `pg_class` holds both.
    //
    // An absent name is the engine's to choose: it is minted here, where a declared
    // one is tested, and steps around a taken name rather than failing on it
    // (`choose_index_name`).
    let name = match name {
        Some(name) if schema.relation_name_taken(name) => {
            if if_not_exists {
                return Ok(QueryResult::ddl("CREATE INDEX"));
            }
            return Err(crate::CatalogError::IndexAlreadyExists {
                index: name.to_string(),
            }
            .into());
        }
        Some(name) => name.to_string(),
        None => choose_index_name(schema, &table_obj.name, &key),
    };

    // The catalog `Index` value is assembled **here**, before the last two gates and
    // before the OID, so the build-time uniqueness scan can key through the write
    // path's own preparation. Nothing is written by building it: `oid` is a
    // placeholder until every check has passed — and the placeholder is kept out of
    // storage by the type rather than by this comment. `PreparedIndex.index_id`
    // is an `Option<u32>` that this `0` maps to `None` (no real OID is below 16384), so
    // the two readers that probe storage with it have to name the case and both refuse
    // loud; a probe against index `0` would answer "no rows", which a uniqueness check
    // reads as "no conflict" and writes on.
    let mut index = Index {
        name,
        table_name: table.to_string(),
        key,
        uniqueness: Uniqueness::new(unique, enforced),
        predicate,
        nulls_not_distinct,
        oid: 0,
    };

    // **18.6's index build**, which raises `could not create unique index … is
    // duplicated` (`23505`) from inside `index_create` — after the name conflict and
    // the `IF NOT EXISTS` skip it shares that call with, and therefore here. Without
    // it a `CREATE UNIQUE INDEX` over already-duplicate rows was accepted, the index
    // simply never held, and the engines went on to disagree about the table's *rows*:
    // 18.6 creates no index and takes the next duplicate, pglet refuses it `23505`.
    // The `ADD CONSTRAINT UNIQUE` / `ADD PRIMARY KEY` routes have always had the gate
    // (`validate_existing_unique`); this route is the one that did not.
    //
    // The build's size refusals straddle that `23505`, as 18.6's do: an entry past
    // `index_form_tuple`'s 8 191 bytes is refused as the heap is read, before the sort
    // that finds a duplicate, and one past the method's item limit as the sorted entries
    // are loaded, after it (`scan_index_build_sizes`).
    let mut sizes = crate::executor::constraints::scan_index_build_sizes(
        storage,
        table_obj,
        &index,
        &schema.enums,
    )?;
    sizes.raise_before_duplicates()?;
    if index.is_enforced() {
        crate::executor::constraints::validate_index_build_unique(
            storage,
            table_obj,
            &index,
            &schema.enums,
        )?;
    }
    sizes.raise_after_duplicates()?;
    sizes.raise_page_bound()?;

    // pglet's own restriction, standing in for **no** Postgres error, so it runs after
    // every check Postgres has — the name conflict, the `IF NOT EXISTS` skip and the
    // build's `23505` included.
    validate_btree_key_type(method, &index, table_obj)?;

    // The second pglet-only over-rejection on this statement, here for the same
    // reason and no other: an index expression the deparser cannot render would take
    // the **whole database's** dump down (`pglet::dump` renders every persisted
    // expression through one `Result`), and 18.6 accepts every such expression. Run
    // with the three checks above (`validate_index_exprs`, which stand in for real
    // 18.6 errors and therefore precede the name), it turned
    // `CREATE INDEX IF NOT EXISTS <a taken name> ON t ((iv + interval '1 day'))` into
    // a `0A000` where 18.6 skips and reports success — measured, the same trap
    // `validate_btree_key_type` was split out of `validate_index_method` for.
    //
    // Its order against `validate_btree_key_type` is **not** observable against 18.6,
    // which accepts both families: they can co-fire only on a non-unique partial index
    // over a bare container key column whose predicate carries an interval, and this
    // one is last because it is the newer arrival, not because anything measures it.
    validate_index_exprs_render(&index.key, index.predicate.as_ref())?;

    // The number the OID counter would hand out, read **without** advancing it:
    // `IndexSpec` is keyed by the OID, so the storage build below cannot be reached
    // without a number, and that build is the statement's last fallible step. Writing
    // the counter here made a *rejected* `CREATE INDEX` bump the persisted `next_oid`,
    // and a repeated idempotent `CREATE INDEX IF NOT EXISTS` consume one per run:
    // benign (the burned number was monotonic and never came back) but a catalog
    // mutation on a statement that changes nothing, which the autocommit path cannot
    // undo. Reading instead of writing gives the number *back* to the next statement,
    // which is a property this deliberately drops and `next_object_oid`'s doc says why
    // it is safe to. `execute_create_type` reads the counter the same way and for the
    // same reason.
    let oid = next_object_oid(schema);
    index.oid = oid;

    // Build the storage spec against the borrowed catalog table (inserting the index
    // into the catalog would re-borrow `schema` mutably).
    let spec = index_spec(&index, get_table_or_err(schema, table)?, &schema.enums)?;

    // Build + back-fill the index data in storage (parses + caches the predicate).
    // **The last fallible step, and it runs before every catalog write**: the build
    // can still fail where no gate above reads — a `jsonb` GIN token longer than a
    // page carries as a key, which the build scan above does not size (an expression
    // key that raises on a row, `(10 / a)` over a zero, is refused above, by the entry
    // sizing) — and the autocommit path rolls back storage
    // while leaving `schema` mutated, so a catalog write ahead of it left a durable
    // phantom index whose name was `42P07` forever and whose table the B+tree backend
    // then refused most writes to. Ordering it this way is only sound because
    // `Storage::create_index` is **all-or-nothing** (it is a trait contract, not an
    // observation — see that method's doc): a failed build registers no def and no
    // `table_indexes` entry, so shedding the catalog half here cannot strand a storage
    // half no statement can name.
    storage.create_index(spec)?;

    // Nothing below can fail, so the two catalog writes go last, in either order.
    schema.next_oid = oid + 1;
    schema.create_index(index);

    Ok(QueryResult::ddl("CREATE INDEX"))
}

/// Execute DROP INDEX.
///
/// The index behind a declared `PRIMARY KEY` / `UNIQUE` constraint is **not**
/// droppable — Postgres records an INTERNAL dependency from the index to its
/// constraint, so the drop is refused with `2BP01` even under `CASCADE` (the
/// constraint has to go instead). pglet had no constraint marker and so dropped it
/// happily, which silently un-enforced the key (duplicate rows, surviving a restart)
/// *and* erased it from the dump — `classify_index` re-derives PK/UNIQUE from the
/// index, so a dumped table came back with neither the key nor a way to notice.
///
/// A standalone `CREATE UNIQUE INDEX` carries no marker and still drops freely, which
/// is exactly Postgres's distinction.
pub fn execute_drop_index<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    names: &[String],
    if_exists: bool,
) -> Result<QueryResult> {
    // Phase 1: every name resolved and kind-checked positionally. As on `DROP TABLE`:
    // `DropErrorMsgWrongType` is reached by a name that *resolves* to the wrong kind,
    // so it beats the `IF EXISTS` skip — `DROP INDEX [IF EXISTS] <a sequence>` and
    // `… <a table>` are both `42809` (`"x" is not an index`), not `42704` and not a
    // no-op (measured on 18.6). A **table** is the reachable half this check first
    // missed: it read only `sequences`, so `DROP INDEX IF EXISTS <a table>` skipped and
    // reported success.
    let (targets, notices) = resolve_drop_targets(schema, names, RelationKind::Index, if_exists)?;

    // Phase 2: a constraint's index is refused, with or without CASCADE. The
    // constraint's name *is* the index's name (see `ConstraintKind`). It is a *phase-2*
    // check, so a later name's phase-1 failure outranks it — `DROP INDEX <a PK's
    // index>, <a missing name>` is `42704` on 18.6, not `2BP01`.
    //
    // The storage ids are collected in the same read-only window, so phase 3 holds no
    // `schema` borrow.
    let mut index_ids: Vec<u32> = Vec::with_capacity(targets.len());
    for name in &targets {
        // Unreachable: phase 1 has already established that every survivor is an
        // index. Tolerated rather than `expect`ed — this is library code.
        let Some(index) = schema.get_index(name) else {
            continue;
        };
        if index.constraint_kind().is_some() {
            return Err(Error::IndexRequiredByConstraint {
                index: index.name.clone(),
                table: index.table_name.clone(),
            });
        }
        index_ids.push(index.index_id());
    }

    // Phase 3, and the fallible half runs first: `storage.drop_index` is the only
    // failure left, and storage rolls back on the autocommit path where the catalog
    // does not — so every storage removal lands before the first catalog edit.
    for id in index_ids {
        storage.drop_index(id)?;
    }
    for name in &targets {
        schema.drop_index(name);
    }
    Ok(QueryResult::ddl_with_notices("DROP INDEX", notices))
}

/// Execute `ALTER TABLE … DROP COLUMN [IF EXISTS] col [CASCADE | RESTRICT]`.
///
/// Dependent objects follow Postgres's edge kinds (see `executor::dependencies`):
/// this table's own `CHECK`s mentioning the column, its own child-side `FOREIGN
/// KEY`s, and the **sequence the column owns** are all **auto-dropped**, while any FK
/// *referencing* `table.column` — from another table or from this one — and any
/// column default drawing from that sequence *from outside the deletion set* raise
/// `2BP01` unless `CASCADE`. Each auto-drop closes an accepted-then-broken state: a
/// left-behind CHECK made every later write `42703` (and resurrected itself if the
/// column was re-added), a left-behind FK went inert, and a left-behind sequence
/// burned its name for every future column of that name and dumped to an
/// `ALTER SEQUENCE … OWNED BY` naming a column the script never creates.
///
/// The sequence's deletion set is **column**-scoped here, which a table drop's is not:
/// `CREATE TABLE q (id SERIAL, b INT DEFAULT nextval('q_id_seq'))` drops as a whole
/// table and refuses `DROP COLUMN id`, because `q.b` survives that statement.
///
/// **Check order.** The dependency check runs *before* the index position-drift
/// guard. A referenced column always carries a covering unique index, so the guard
/// would otherwise always win and report `0A000` where Postgres reports `2BP01` — the
/// FK is the reason the drop is refused, and Postgres names it. The consequence is
/// that `DROP COLUMN … CASCADE` on a *referenced* column still fails, now on the
/// drift guard (`0A000`), where Postgres succeeds: pglet does not rebuild indexes on
/// column drop, so the covering index cannot follow the constraint out.
pub fn execute_alter_table_drop_column<S: Storage>(
    schema: &mut Schema,
    storage: &mut S,
    table_name: &str,
    column_name: &str,
    if_exists: bool,
    cascade: bool,
) -> Result<QueryResult> {
    // The target's existence **and kind** (`ATSimplePermissions`: `ALTER action DROP
    // COLUMN cannot be performed on relation "s"` → `42809` for a sequence).
    let table = get_table_or_err(schema, table_name)?;

    // Find column index (immutable borrow first, so the index-drift guard below can
    // also read `schema.indexes` before any mutation).
    let Some(idx) = table.columns.iter().position(|c| c.name == column_name) else {
        return if if_exists {
            Ok(QueryResult::ddl("ALTER TABLE"))
        } else {
            Err(crate::CatalogError::ColumnNotFound {
                table: table_name.to_string(),
                column: column_name.to_string(),
            }
            .into())
        };
    };

    // Dependency resolution first — see the doc comment on check order.
    let target = DropTarget::Column {
        table: table_name,
        column: column_name,
    };
    let dependents = Dependents::resolve(schema, target);
    dependents.ensure_droppable(target, cascade)?;

    // Index position-drift guard. pglet keys indexes by resolved row *position*
    // (Postgres uses stable attnums), so dropping a column at/below an indexed
    // position would shift the key onto the wrong column. We don't rebuild indexes
    // on drop, so fail loud rather than silently corrupt the index instead of
    // returning wrong rows on a later probe.
    if let Some((index, reason)) = schema.indexes.values().find_map(|index| {
        (index.table_name == table_name)
            .then(|| index_affected_by_column_drop(index, table, idx))
            .flatten()
            .map(|reason| (index, reason))
    }) {
        return Err(Error::NotImplemented(format!(
            "cannot DROP COLUMN \"{column_name}\" on \"{table_name}\": index \"{}\" {reason} \
             (pglet does not rebuild indexes on column drop)",
            index.name
        )));
    }

    // Shed the dependent constraints the column takes with it (and, under CASCADE,
    // the blocking ones) before the column itself goes.
    dependents.drop_from_catalog(schema, target, cascade);

    // Remove the column from the catalog and renumber the rest.
    let table =
        schema
            .get_table_mut(table_name)
            .ok_or_else(|| crate::CatalogError::TableNotFound {
                table: table_name.to_string(),
            })?;
    table.columns.remove(idx);
    for (i, col) in table.columns.iter_mut().enumerate() {
        col.position = (i + 1) as i16;
    }

    // Remove the column from storage. Propagate the error (a swallowed failure here
    // would diverge the catalog from the stored rows) rather than dropping it.
    storage.drop_column(table_name, idx)?;

    Ok(QueryResult::ddl("ALTER TABLE"))
}

/// Why dropping the column at row position `idx` would invalidate `index`'s stored
/// key/predicate positions, or `None` when it would not — the phrase the refusal names.
///
/// A plain index is affected when any key column sits at or after `idx` (the
/// dropped column itself, or one whose position shifts). An expression or partial
/// index evaluates against the whole row by position, so any drop can shift a
/// referenced column — treated as affected conservatively (correct-or-loud over a
/// non-spine path: pglet never rebuilds indexes on column drop).
///
/// **The reason is returned rather than folded into one sentence** because those are
/// three different verdicts and the single message asserted the narrowest for all of
/// them: over `dcp(a, b, c)` under `CREATE INDEX dcp_pred ON dcp (a) WHERE c > 0`,
/// dropping `b` was refused as *"index dcp_pred keys on this column or one after it"* —
/// false, since the key is at position 0, before the drop, and it denies the real reason,
/// the predicate at position 2. That the conservative arms are also an **over-rejection**
/// — 18.6 accepts all three of those drops and keeps enforcing the index — is
/// `EDGES.md` row 108; what is fixed here is a message that told the reader the wrong
/// thing about their own schema.
fn index_affected_by_column_drop(index: &Index, table: &Table, idx: usize) -> Option<&'static str> {
    if index.predicate.is_some() {
        return Some("has a partial predicate, which reads the row by position");
    }
    match &index.key {
        IndexKey::Columns(cols) => cols
            .iter()
            .any(|name| {
                table
                    .columns
                    .iter()
                    .position(|c| c.name == *name)
                    .is_none_or(|pos| pos >= idx)
            })
            .then_some("keys on this column or one after it"),
        IndexKey::Exprs(_) => Some("has an expression key, which reads the row by position"),
    }
}

#[cfg(test)]
mod dedup_tests {
    use super::dedup_unique_constraints;
    use crate::planner::UniqueConstraint;

    fn uc(is_primary: bool, name: Option<&str>, columns: &[&str]) -> UniqueConstraint {
        UniqueConstraint {
            columns: columns.iter().map(|c| (*c).to_string()).collect(),
            is_primary,
            name: name.map(ToString::to_string),
        }
    }

    /// The shape a redundant specification collapses to: one constraint, and the
    /// *statement order* it was written in must not decide which survives.
    #[test]
    fn a_primary_key_absorbs_an_identical_unique_from_either_side() {
        for constraints in [
            vec![uc(true, None, &["a"]), uc(false, None, &["a"])],
            vec![uc(false, None, &["a"]), uc(true, None, &["a"])],
        ] {
            let kept = dedup_unique_constraints(&constraints);
            assert_eq!(kept.len(), 1, "the two describe one index");
            assert!(
                kept[0].is_primary,
                "the primary key is kept in preference — a UNIQUE written first must not \
                 absorb it and demote the constraint to UNIQUE"
            );
        }
    }

    /// `if (priorindex->idxname == NULL) priorindex->idxname = index->idxname;` — the
    /// survivor takes the name of the duplicate it absorbed, so a `CONSTRAINT myu
    /// UNIQUE (a)` merged into an unnamed `PRIMARY KEY` yields a primary key called
    /// `myu` and not `t_pkey`.
    #[test]
    fn an_absorbed_name_transfers_onto_an_unnamed_survivor() {
        let kept =
            dedup_unique_constraints(&[uc(true, None, &["a"]), uc(false, Some("myu"), &["a"])]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name.as_deref(), Some("myu"));
        assert!(kept[0].is_primary);
    }

    /// ...but only onto an *unnamed* one: a survivor that already carries a declared
    /// name keeps it (Postgres uses "at least one of" the names, the first).
    #[test]
    fn a_named_survivor_keeps_its_own_name() {
        let kept = dedup_unique_constraints(&[
            uc(false, Some("n1"), &["a"]),
            uc(false, Some("n2"), &["a"]),
        ]);
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].name.as_deref(), Some("n1"));
    }

    /// Key **order** is part of the index specification, so a reordered composite is a
    /// different index and both survive. A set-based comparison here would silently
    /// drop one of a table's two indexes.
    #[test]
    fn a_reordered_composite_is_not_a_duplicate() {
        let kept =
            dedup_unique_constraints(&[uc(false, None, &["a", "b"]), uc(false, None, &["b", "a"])]);
        assert_eq!(kept.len(), 2);
    }

    /// Distinct constraints pass through untouched, in order.
    #[test]
    fn distinct_specifications_are_all_kept_in_order() {
        let kept = dedup_unique_constraints(&[
            uc(false, None, &["b"]),
            uc(true, None, &["a"]),
            uc(false, None, &["c"]),
        ]);
        let columns: Vec<&str> = kept.iter().map(|k| k.columns[0].as_str()).collect();
        // The primary key is hoisted to the head; the rest keep their relative order,
        // matching `finalindexlist`'s construction.
        assert_eq!(columns, vec!["a", "b", "c"]);
    }
}

#[cfg(test)]
mod mint_tests {
    use super::ConstraintNames;
    use crate::catalog::{
        CheckConstraint, Column, ConstraintKind, ForeignKey, Index, IndexKey, ReferentialAction,
        Schema, Table, Uniqueness,
    };
    use crate::planner::{BinaryOp, Expr, ForeignKeyDef, Literal, UniqueConstraint};
    use crate::types::TypeOid;

    fn column(name: &str) -> Column {
        Column {
            name: name.to_string(),
            type_oid: TypeOid::Int4,
            nullable: true,
            position: 1,
            default: None,
            enum_type: None,
            numeric_typmod: None,
            varchar_length: None,
            temporal_precision: None,
            identity: None,
        }
    }

    fn table(name: &str) -> Table {
        Table {
            name: name.to_string(),
            schema: "public".to_string(),
            columns: vec![column("a"), column("b")],
            oid: 1,
            checks: Vec::new(),
            foreign_keys: Vec::new(),
        }
    }

    fn check(name: &str) -> CheckConstraint {
        CheckConstraint {
            name: name.to_string(),
            expr: Expr::BinaryOp {
                left: Box::new(Expr::Column {
                    table: None,
                    name: "a".to_string(),
                }),
                op: BinaryOp::Gt,
                right: Box::new(Expr::Literal(Literal::Int(0))),
            },
        }
    }

    fn uc(is_primary: bool, name: Option<&str>, columns: &[&str]) -> UniqueConstraint {
        UniqueConstraint {
            columns: columns.iter().map(|c| (*c).to_string()).collect(),
            is_primary,
            name: name.map(ToString::to_string),
        }
    }

    /// An empty one-table schema (`t(a, b)`), plus whatever the caller adds.
    fn schema() -> Schema {
        let mut s = Schema::new();
        s.create_table(table("t"));
        s
    }

    #[test]
    fn a_declared_name_is_verbatim_and_a_collision_is_the_user_error() {
        let mut s = schema();
        s.get_table_mut("t").unwrap().checks.push(check("taken"));
        let mut names = ConstraintNames::new(&s, "t");
        assert_eq!(
            names.mint_constraint(Some("fresh"), "a", "check").unwrap(),
            "fresh",
            "a declared name is never disambiguated — it is taken as written"
        );
        let err = names
            .mint_constraint(Some("taken"), "a", "check")
            .expect_err("a declared name already on this table is 42710");
        assert_eq!(err.sqlstate(), "42710");
        // …and the name this statement just claimed is visible to the next constraint
        // (Postgres's `CommandCounterIncrement`), so the same declared name twice in
        // one `CREATE TABLE` collides even with nothing in the catalog.
        assert_eq!(
            names
                .mint_constraint(Some("fresh"), "a", "check")
                .unwrap_err()
                .sqlstate(),
            "42710"
        );
    }

    #[test]
    fn an_auto_name_is_disambiguated_rather_than_refused() {
        let mut s = schema();
        s.get_table_mut("t")
            .unwrap()
            .checks
            .push(check("t_a_check"));
        let mut names = ConstraintNames::new(&s, "t");
        // The base is taken in the catalog, so the digit is appended — where the same
        // name *declared* would have been 42710. Postgres never lets one of its own
        // generated names fail a statement.
        assert_eq!(
            names.mint_constraint(None, "a", "check").unwrap(),
            "t_a_check1"
        );
        // The claimed one is likewise stepped over, not collided with.
        assert_eq!(
            names.mint_constraint(None, "a", "check").unwrap(),
            "t_a_check2"
        );
    }

    #[test]
    fn the_two_sides_search_different_scopes() {
        // `other` carries `t_a_check`; `t` carries nothing.
        let mut s = schema();
        s.create_table(table("other"));
        s.get_table_mut("other")
            .unwrap()
            .checks
            .push(check("t_a_check"));

        // Declared: per-relation (`ConstraintNameIsUsed`) — a name on another table is
        // free. This is the direction no fixture asserted, and inverting it rejects the
        // ordinary two-tables-one-constraint-name schema while leaving the corpus green.
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint(Some("t_a_check"), "a", "check")
                .unwrap(),
            "t_a_check"
        );

        // Auto: schema-wide (`ChooseConstraintName` over `pg_constraint (conname,
        // connamespace)`) — the very same name on the very same other table pushes the
        // candidate aside. "Deliberately not the same test."
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint(None, "a", "check")
                .unwrap(),
            "t_a_check1"
        );
    }

    /// The names 18.6 chose for these (`ddl/constraint_auto_name_*` hold the same
    /// answers differentially): each kind is cut to 63 bytes, a primary key's and a
    /// table-level `CHECK`'s over the table name alone, and two names that meet only
    /// after the cut take the digit on the label, inside the cut.
    #[test]
    fn an_auto_name_is_cut_to_63_bytes_and_the_digit_is_inside_the_cut() {
        let long = format!("t{}", "a".repeat(59));
        let mut s = Schema::new();
        s.create_table(table(&long));
        let mut names = ConstraintNames::new(&s, &long);
        assert_eq!(
            names
                .mint_constraint_index(&uc(true, None, &["a"]))
                .unwrap(),
            format!("t{}_pkey", "a".repeat(57))
        );
        assert_eq!(
            names.mint_constraint(None, "", "check").unwrap(),
            format!("t{}_check", "a".repeat(56))
        );
        // A short second name is kept whole; the table's name gives up the bytes.
        assert_eq!(
            names.mint_constraint(None, "a_b", "fkey").unwrap(),
            format!("t{}_a_b_fkey", "a".repeat(53))
        );

        // Two column names that differ in their last byte, past where the cut lands.
        let table_name = format!("t{}", "a".repeat(39));
        let (x, y) = (
            format!("c{}x", "b".repeat(39)),
            format!("c{}y", "b".repeat(39)),
        );
        let mut s = Schema::new();
        s.create_table(table(&table_name));
        let mut names = ConstraintNames::new(&s, &table_name);
        let first = names
            .mint_constraint_index(&uc(false, None, &[&x]))
            .unwrap();
        assert_eq!(
            first,
            format!("t{}_c{}_key", "a".repeat(28), "b".repeat(28))
        );
        let second = names
            .mint_constraint_index(&uc(false, None, &[&y]))
            .unwrap();
        assert_eq!(
            second,
            format!("t{}_c{}_key1", "a".repeat(28), "b".repeat(27))
        );
        let check = names.mint_constraint(None, &x, "check").unwrap();
        assert_eq!(
            check,
            format!("t{}_c{}_check", "a".repeat(27), "b".repeat(27))
        );
        assert_eq!(
            names.mint_constraint(None, &y, "check").unwrap(),
            format!("t{}_c{}_check1", "a".repeat(27), "b".repeat(26))
        );
        for name in [&first, &second, &check] {
            assert_eq!(name.len(), 63);
        }
    }

    #[test]
    fn an_auto_constraint_name_ignores_plain_relations_but_an_index_name_does_not() {
        let mut s = schema();
        // A standalone `CREATE INDEX t_a_fkey ON t (b)` — a relation, not a constraint.
        s.create_index(Index {
            name: "t_a_fkey".to_string(),
            table_name: "t".to_string(),
            key: IndexKey::Columns(vec!["b".to_string()]),
            uniqueness: Uniqueness::NonUnique,
            predicate: None,
            nulls_not_distinct: false,
            oid: 100,
        });
        // A CHECK/FK auto-name reads only `pg_constraint`, so it takes the base.
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint(None, "a", "fkey")
                .unwrap(),
            "t_a_fkey"
        );
        // A PK/UNIQUE auto-name must also clear the relation namespace, because its
        // index carries the name (`ChooseRelationName`).
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint_index(&uc(false, None, &["a"]))
                .unwrap(),
            "t_a_key"
        );
        s.create_index(Index {
            name: "t_a_key".to_string(),
            table_name: "t".to_string(),
            key: IndexKey::Columns(vec!["a".to_string()]),
            uniqueness: Uniqueness::NonUnique,
            predicate: None,
            nulls_not_distinct: false,
            oid: 101,
        });
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint_index(&uc(false, None, &["a"]))
                .unwrap(),
            "t_a_key1"
        );
    }

    #[test]
    fn a_constraint_index_checks_the_relation_namespace_first() {
        let mut s = schema();
        // The case that actually pins the *order*: a name held by a constraint-marked
        // index on this table is in **both** namespaces, so only the order decides
        // which error comes out. `index_create` tests the relation namespace first, so
        // it is `42P07` — measured on 18.6 (`CREATE TABLE ns1 (a INT, b INT, CONSTRAINT
        // shared UNIQUE (b)); ALTER TABLE ns1 ADD CONSTRAINT shared UNIQUE (a);` →
        // `42P07 relation "shared" already exists`, where the CHECK spelling of the same
        // name is `42710`). A name in only one namespace cannot see the order at all —
        // the first version of this test used a plain index and passed with the checks
        // swapped.
        s.create_index(Index {
            name: "both".to_string(),
            table_name: "t".to_string(),
            key: IndexKey::Columns(vec!["b".to_string()]),
            uniqueness: Uniqueness::for_constraint(ConstraintKind::Unique),
            predicate: None,
            nulls_not_distinct: false,
            oid: 99,
        });
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint_index(&uc(false, Some("both"), &["a"]))
                .unwrap_err()
                .sqlstate(),
            "42P07",
            "the relation namespace is tested first, so it supplies the error"
        );

        // A declared name colliding with a *plain* index is `42P07` too — that one is in
        // the relation namespace only, so it says nothing about the order.
        s.create_index(Index {
            name: "rel".to_string(),
            table_name: "t".to_string(),
            key: IndexKey::Columns(vec!["b".to_string()]),
            uniqueness: Uniqueness::NonUnique,
            predicate: None,
            nulls_not_distinct: false,
            oid: 100,
        });
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint_index(&uc(true, Some("rel"), &["a"]))
                .unwrap_err()
                .sqlstate(),
            "42P07"
        );
        // …while a collision with a CHECK on this table — which is in the constraint
        // namespace only — is `42710`.
        s.get_table_mut("t").unwrap().checks.push(check("ck"));
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint_index(&uc(true, Some("ck"), &["a"]))
                .unwrap_err()
                .sqlstate(),
            "42710"
        );
        // A CHECK named after that plain index is accepted on both engines — the
        // constraint namespace is the only one a CHECK enters.
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint(Some("rel"), "a", "check")
                .unwrap(),
            "rel"
        );
    }

    #[test]
    fn a_composite_fk_auto_name_joins_every_child_column() {
        let fk = ForeignKeyDef {
            name: None,
            columns: vec!["a".to_string(), "b".to_string()],
            parent_table: "p".to_string(),
            parent_columns: vec!["x".to_string(), "y".to_string()],
            on_delete: ReferentialAction::NoAction,
            on_update: ReferentialAction::NoAction,
        };
        // `ChooseForeignKeyConstraintNameAddition` joins the child columns with `_`;
        // the single-column case (`planner::tests`) cannot see the join at all.
        assert_eq!(
            ConstraintNames::new(&schema(), "t")
                .mint_foreign_key(&fk)
                .unwrap(),
            "t_a_b_fkey"
        );

        let mut s = schema();
        s.get_table_mut("t").unwrap().foreign_keys.push(ForeignKey {
            name: "t_a_b_fkey".to_string(),
            columns: vec!["a".to_string(), "b".to_string()],
            parent_table: "p".to_string(),
            parent_columns: vec!["x".to_string(), "y".to_string()],
            on_delete: ReferentialAction::NoAction,
            on_update: ReferentialAction::NoAction,
        });
        assert_eq!(
            ConstraintNames::new(&s, "t").mint_foreign_key(&fk).unwrap(),
            "t_a_b_fkey1"
        );
    }

    #[test]
    fn a_pk_and_a_unique_take_different_auto_bases() {
        let s = schema();
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint_index(&uc(true, None, &["a"]))
                .unwrap(),
            "t_pkey"
        );
        assert_eq!(
            ConstraintNames::new(&s, "t")
                .mint_constraint_index(&uc(false, None, &["a", "b"]))
                .unwrap(),
            "t_a_b_key"
        );
        // A constraint elsewhere in the schema pushes a PK/UNIQUE auto-name too, via
        // the `constraint_name_exists` term alone (`ChooseRelationName`'s
        // `isconstraint` pass finishes with `ConstraintNameExists`). The seed is a
        // **CHECK** on the other table on purpose: seeding an index named `t_pkey`
        // would be caught by `relation_taken` — `Schema::relation_name_taken` is
        // global over every index — so it would pass with the term deleted and assert
        // nothing about the scope it claims to.
        let mut s2 = schema();
        s2.create_table(table("other"));
        s2.get_table_mut("other")
            .unwrap()
            .checks
            .push(check("t_pkey"));
        assert_eq!(
            ConstraintNames::new(&s2, "t")
                .mint_constraint_index(&uc(true, None, &["a"]))
                .unwrap(),
            "t_pkey1"
        );
    }
}

#[cfg(test)]
mod relation_name_tests {
    use super::{choose_index_name, choose_relation_name, make_object_name};
    use crate::catalog::{IndexKey, Schema, Sequence};
    use crate::planner::{BinaryOp, Expr, Literal};

    fn column(name: &str) -> Expr {
        Expr::Column {
            table: None,
            name: name.to_string(),
        }
    }

    /// The names 18.6 chose for these parts (`ddl/create_index_unnamed_long_names_are_cut`
    /// holds the same answers differentially). The longer name gives up bytes first, down
    /// to the shorter one's length, then both — never the tail of the joined string, and
    /// never the label.
    #[test]
    fn the_longer_name_is_cut_first_and_the_label_never() {
        let table = format!("t{}", "a".repeat(39));
        let long = format!("c{}", "b".repeat(49));
        let even = format!("c{}", "b".repeat(39));
        assert_eq!(
            make_object_name(&table, &long, "idx"),
            format!("t{}_c{}_idx", "a".repeat(28), "b".repeat(28))
        );
        assert_eq!(
            make_object_name(&table, &even, "idx"),
            format!("t{}_c{}_idx", "a".repeat(28), "b".repeat(28))
        );
        assert_eq!(make_object_name(&table, &even, "idx").len(), 63);
        // A digit on the label takes its byte from the names, not from the limit.
        assert_eq!(
            make_object_name(&table, &even, "idx1"),
            format!("t{}_c{}_idx1", "a".repeat(28), "b".repeat(27))
        );
        assert_eq!(make_object_name("t", "a", "idx"), "t_a_idx");
    }

    /// The digit steps around the relation namespace — a sequence here — and lands on the
    /// label, inside the cut.
    #[test]
    fn a_taken_name_moves_the_digit_onto_the_label() {
        let mut schema = Schema::new();
        assert_eq!(choose_relation_name(&schema, "t", "a", "idx"), "t_a_idx");
        schema
            .sequences
            .insert("t_a_idx".to_string(), Sequence::new(1));
        schema
            .sequences
            .insert("t_a_idx1".to_string(), Sequence::new(1));
        assert_eq!(choose_relation_name(&schema, "t", "a", "idx"), "t_a_idx2");
    }

    /// A key part's name: a bare column's own, an expression's output-column name with
    /// `expr` for a nameless one, and a digit of its own on a name the key already holds.
    #[test]
    fn key_parts_are_named_and_repeats_take_a_digit() {
        let schema = Schema::new();
        let key = IndexKey::Columns(vec!["x".into(), "y".into(), "x".into(), "x".into()]);
        assert_eq!(choose_index_name(&schema, "t", &key), "t_x_y_x1_x2_idx");

        let sum = Expr::BinaryOp {
            left: Box::new(column("a")),
            op: BinaryOp::Add,
            right: Box::new(Expr::Literal(Literal::Int(1))),
        };
        let key = IndexKey::Exprs(vec![
            column("a1"),
            sum.clone(),
            column("a"),
            sum,
            column("a"),
        ]);
        assert_eq!(
            choose_index_name(&schema, "t", &key),
            "t_a1_expr_a_expr1_a2_idx"
        );

        // A column that is itself named `?column?` keeps its name: only a nameless
        // expression is `expr`.
        let key = IndexKey::Exprs(vec![column("?column?")]);
        assert_eq!(choose_index_name(&schema, "t", &key), "t_?column?_idx");
    }
}

#[cfg(test)]
mod backfill_volatility_tests {
    use super::{ColumnDefault, is_per_row_volatile};
    use crate::planner::{BuiltinFn, Expr};
    use crate::types::Value;

    /// Enumerates [`is_per_row_volatile`] over **every** `ColumnDefault` arm plus `None`,
    /// so a variant added without a decision fails to compile here rather than
    /// silently joining the stable side.
    ///
    /// This is the deterministic half of the `clock_timestamp()` guard. The
    /// differential fixture asserts the *consequence* (distinct instants land in the
    /// backfilled rows), which depends on the clock advancing during the loop; this
    /// asserts the *mechanism*, and is what goes red on a revert of the classifier
    /// regardless of how fast the box is.
    ///
    /// Each verdict is 18.6's, measured by `ALTER TABLE … ADD COLUMN c … DEFAULT <d>`
    /// over five rows and counting `DISTINCT`: 1 for `now()`,
    /// `transaction_timestamp()`, `statement_timestamp()` and `CURRENT_TIMESTAMP`
    /// (transaction- or statement-stable, and one `ALTER` is one of each), 3–4 for
    /// `clock_timestamp()` on one idle host (a count the host's speed sets), 5 for
    /// `gen_random_uuid()`.
    #[test]
    fn only_the_three_volatile_defaults_resolve_per_row() {
        // The `Expr` arm's verdict is a **walk**, so it appears on both sides: a
        // volatile leaf anywhere makes the whole expression volatile, and an
        // expression built only from stable leaves stays stable. Reading the top node
        // alone would put `nextval('s') + 1` on the stable side and backfill one value
        // into every row (18.6 rewrites per row — measured, two inserts over
        // `DEFAULT nextval('s') + 1` answer `2` then `3`).
        let plus_one = |leaf: Expr| {
            ColumnDefault::Expr(Expr::BinaryOp {
                left: Box::new(leaf),
                op: crate::planner::BinaryOp::Add,
                right: Box::new(Expr::Literal(crate::planner::Literal::Int(1))),
            })
        };
        for default in [
            ColumnDefault::RandomUuid,
            ColumnDefault::NextVal("s".to_string()),
            ColumnDefault::ClockTimestamp,
            plus_one(Expr::SequenceFn {
                func: crate::planner::SequenceFunc::Nextval,
                seq: "s".to_string(),
                args: Vec::new(),
            }),
            plus_one(Expr::Builtin(BuiltinFn::ClockTimestamp)),
            plus_one(Expr::Builtin(BuiltinFn::RandomUuid)),
        ] {
            assert!(
                is_per_row_volatile(Some(&default)),
                "{default:?} must resolve per backfilled row"
            );
        }
        for default in [
            ColumnDefault::Literal(Value::Int8(1)),
            ColumnDefault::Literal(Value::Null),
            ColumnDefault::Now,
            ColumnDefault::TransactionTimestamp,
            ColumnDefault::StatementTimestamp,
            ColumnDefault::CurrentTimestamp,
            ColumnDefault::CurrentDate,
            ColumnDefault::CurrentTime,
            plus_one(Expr::Builtin(BuiltinFn::Now)),
            plus_one(Expr::Builtin(BuiltinFn::TransactionTimestamp)),
            plus_one(Expr::Builtin(BuiltinFn::StatementTimestamp)),
            plus_one(Expr::Builtin(BuiltinFn::CurrentTimestamp)),
            plus_one(Expr::Builtin(BuiltinFn::CurrentDate)),
            plus_one(Expr::Builtin(BuiltinFn::CurrentTime)),
            plus_one(Expr::Literal(crate::planner::Literal::Int(1))),
        ] {
            assert!(
                !is_per_row_volatile(Some(&default)),
                "{default:?} is stable within one statement — one value fills every row"
            );
        }
        assert!(
            !is_per_row_volatile(None),
            "no default fills every row with NULL"
        );

        // Exhaustiveness: this match has no wildcard, so a new `ColumnDefault` variant
        // breaks the build here and forces the volatility decision to be made.
        let all_arms_are_covered = |d: &ColumnDefault| match d {
            ColumnDefault::Literal(_)
            | ColumnDefault::Now
            | ColumnDefault::CurrentDate
            | ColumnDefault::RandomUuid
            | ColumnDefault::NextVal(_)
            | ColumnDefault::CurrentTime
            | ColumnDefault::TransactionTimestamp
            | ColumnDefault::StatementTimestamp
            | ColumnDefault::ClockTimestamp
            | ColumnDefault::CurrentTimestamp
            | ColumnDefault::Expr(_) => (),
        };
        all_arms_are_covered(&ColumnDefault::Now);
    }
}

#[cfg(test)]
mod enums_moved_tests {
    use super::enums_moved_by_restore;
    use crate::catalog::{EnumType, Schema};

    /// A catalog carrying only enums — the tables and indexes
    /// [`enums_moved_by_restore`] never reads. Its OIDs are distinct and arbitrary: the
    /// comparison is on `labels`, so an enum whose OID moved under a member list that did
    /// not is *not* named, which is right because `IndexSpec::column_enum_orders` holds
    /// the labels and not the OID.
    fn schema_with(enums: &[(&str, &[&str])]) -> Schema {
        let mut schema = Schema::default();
        for (i, (name, labels)) in enums.iter().enumerate() {
            schema.enums.insert(
                (*name).to_string(),
                EnumType {
                    #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
                    oid: 16_400 + i as i32,
                    labels: labels.iter().map(|l| (*l).to_string()).collect(),
                },
            );
        }
        schema
    }

    /// The answer as a caller consumes it — a set of names. [`rebuild_indexes_over_enums`]
    /// iterates it and each `rebuild_indexes_over_enum` re-derives from the live catalog
    /// and the current rows, so a resync that succeeds lands the same specs whichever
    /// order the two loops produced. That is not the same as unobservable: the iteration
    /// is `for key in moved { …? }`, so where two of the names carry a rebuild that
    /// fails, the order decides which failure surfaces. Pinning it here would redden a
    /// rewrite of the derivation that no *successful* resync could tell apart, which is
    /// why this helper sorts.
    ///
    /// [`rebuild_indexes_over_enums`]: super::rebuild_indexes_over_enums
    fn moved(schema: &Schema, snapshot: &Schema) -> Vec<String> {
        let mut names = enums_moved_by_restore(schema, snapshot);
        names.sort();
        names
    }

    /// The **direct** test of the derivation: it asserts the names the function returns
    /// rather than a write downstream of them.
    ///
    /// Two mutations were run against the whole workspace
    /// (`cargo test --workspace --no-fail-fast`) and both are recorded as counts rather
    /// than as a claim about what a test *could* see:
    ///
    /// - **The second loop deleted** — a name the snapshot holds and the live catalog
    ///   does not, the `DROP TYPE`-inside-a-block-then-restore route. 1690 passed /
    ///   2 failed, the two being this test and
    ///   `crates/pglet/tests/enum_reorder_index_specs.rs`'s
    ///   `a_rolled_back_drop_type_with_no_re_create_leaves_the_specs_on_the_restored_order`
    ///   — the script written for the same arm, and the only behavioural one that moved.
    /// - **The body replaced by a blanket `snapshot.enums.keys().cloned().collect()`** —
    ///   a slower *correct* implementation as far as any write can see, since re-deriving
    ///   an unmoved spec produces the same bytes. 1691 passed / 1 failed, and the one is
    ///   this test's first assertion: nothing behavioural moved.
    ///
    /// What it does **not** look at: whether any caller runs the answer, whether it runs
    /// it before or after `Schema::restore_from`, and what the answer costs. The first two
    /// are the engine tests' and the third is nobody's.
    #[test]
    fn a_restore_names_the_enums_it_moves_and_no_others() {
        let unmoved: &[&str] = &["cold", "warm"];

        // Nothing moved: the whole point of comparing rather than resyncing, and the
        // case a blanket `snapshot.enums.keys()` gets wrong most loudly.
        let same = schema_with(&[("mood", &["sad", "ok", "happy"]), ("weather", unmoved)]);
        assert!(
            moved(&same, &same.clone()).is_empty(),
            "an ordinary ROLLBACK over an untouched catalog re-derives nothing"
        );

        // Reordered under the block, and a second enum beside it that did not move —
        // the pair that separates the comparison from a blanket resync.
        let reordered = schema_with(&[("mood", &["happy", "ok", "sad"]), ("weather", unmoved)]);
        assert_eq!(
            moved(&reordered, &same),
            vec!["mood".to_string()],
            "only the enum whose member list differs is named"
        );

        // The **second loop**: the block dropped `mood`, so the restore adds a name the
        // live catalog does not hold. Deleting that loop answers `[]` here.
        let dropped = schema_with(&[("weather", unmoved)]);
        assert_eq!(
            moved(&dropped, &same),
            vec!["mood".to_string()],
            "a name only the snapshot holds is a move the restore is about to make"
        );

        // The first loop's other half: the block *created* `mood`, so the restore
        // removes it. Symmetric with the case above and answered by a different loop.
        assert_eq!(
            moved(&same, &dropped),
            vec!["mood".to_string()],
            "a name only the live catalog holds is a move too"
        );

        // Both directions at once, so a body that returns after its first hit is red.
        let two_ways = schema_with(&[("mood", &["happy", "ok", "sad"]), ("gone", &["x"])]);
        assert_eq!(
            moved(&two_ways, &same),
            vec![
                "gone".to_string(),
                "mood".to_string(),
                "weather".to_string()
            ],
            "a name added, a name reordered and a name removed are each named once"
        );

        // A member list that differs only by a **relabel** rather than a reorder is a
        // move as well: the comparison is on the labels, not on their multiset.
        let relabelled = schema_with(&[("mood", &["sad", "ok", "elated"]), ("weather", unmoved)]);
        assert_eq!(
            moved(&relabelled, &same),
            vec!["mood".to_string()],
            "a replaced member label moves every ranking that reads the list"
        );
    }
}
