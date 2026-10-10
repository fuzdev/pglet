//! Sequence functions (`nextval` / `currval` / `setval`) as statement-level
//! expressions.
//!
//! pglet's expression evaluation is deliberately **read-only** (no `Schema`
//! access). A sequence function, though, either mutates sequence state
//! (`nextval`/`setval`) or reads **per-session** state (`currval`), so it cannot
//! ride that path. Instead it is evaluated here, at the statement executor, which
//! owns `&mut Schema` and the per-session [`SequenceSession`] — but only when the
//! function appears as a **top-level** no-FROM `SELECT` projection (and, for
//! nextval/setval, an `INSERT … VALUES(…)` entry). A `SequenceFn` in any nested
//! position fails loud in `eval_expr_full` (the read-only path), preserving the
//! read-only-eval principle.

use std::collections::HashMap;

use crate::catalog::Schema;
use crate::planner::{Expr, Literal, PlannerScope, SequenceFunc};
use crate::types::{ResultShape, TypeOid, Value, opt_type_oid_display_name};
use crate::{Error, Result};

use super::Storage;
use super::ddl::ensure_sequence_relation;
use super::expr::{EvalEnv, ScopeIdentity, eval_expr_env};
use super::helpers::{ProjectionScope, expr_column_descs};
use super::query::eval_expr_subquery;
use super::types::QueryResult;

/// Per-session state for the sequence functions, threaded into statement
/// execution.
///
/// Lives **outside** the shared [`Schema`] (which is global — the server holds one
/// `Mutex<Schema>` behind every connection) because `currval` reports the last
/// `nextval` **in this session**. The server keeps one on each `Connection`, the
/// embed facade one on each `Database`.
#[derive(Debug, Default)]
pub struct SequenceSession {
    /// The last value `nextval` returned per sequence this session, keyed by the
    /// bare sequence name. Presence of a key is what makes `currval` defined.
    currval: HashMap<String, i64>,
}

impl SequenceSession {
    /// A fresh session (no sequence has been advanced yet).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Record `value` as the last value `nextval` returned for `seq` this session.
    fn record_currval(&mut self, seq: &str, value: i64) {
        self.currval.insert(seq.to_string(), value);
    }

    /// The last value `nextval` returned for `seq` this session, or `None` if it
    /// has not been advanced (nor `setval`-with-`is_called` set) here.
    fn currval(&self, seq: &str) -> Option<i64> {
        self.currval.get(seq).copied()
    }

    /// Forget every recorded `currval` — `DISCARD SEQUENCES` (and the `DISCARD ALL`
    /// that contains it), whose whole observable effect this is: a `currval` after
    /// it is `55000` on both engines, exactly as it is on a session that never
    /// advanced the sequence.
    ///
    /// It moves no counter. `nextval` state is database-wide and durable; this map
    /// is the per-session half, and only the session half is discarded.
    pub fn clear_currval(&mut self) {
        self.currval.clear();
    }
}

/// Execute a top-level sequence-function projection (`SELECT nextval/currval/
/// setval(...)`) or advisory-lock projection (`SELECT pg_advisory_xact_lock(…)`),
/// returning its single result row.
///
/// `row` is the projection's single row — the caller (`Plan::Values` dispatch via
/// [`Plan::is_sequence_projection`](crate::planner::Plan::is_sequence_projection) or
/// [`Plan::is_advisory_lock_projection`](crate::planner::Plan::is_advisory_lock_projection))
/// guarantees a single-row projection.
///
/// Each projected expression is a top-level sequence function — evaluated against
/// `&mut schema.sequences` + `session` — a top-level `pg_advisory_xact_lock` call, whose
/// lock the host took as the writer before the statement ran, so it answers here
/// (`void`, or NULL over a NULL argument), or an ordinary expression (a mixed
/// projection like pg_dump's `set_config(...), setval(...)`), evaluated via the usual
/// subquery-capable path. Either function nested inside a larger expression is *not*
/// top-level, so it flows to the ordinary path and fails loud there.
///
/// # Errors
///
/// Fails loud on an unknown sequence (`42P01`), an undefined `currval` (`55000`),
/// or any nested sequence function.
pub(super) fn execute_sequence_projection<S: Storage>(
    row: &[Expr],
    schema: &mut Schema,
    storage: &S,
    session: &mut SequenceSession,
    env: EvalEnv<'_>,
) -> Result<QueryResult> {
    // Column descriptors come from the shared projection walk (which names/types a
    // `SequenceFn` as `nextval`/`currval`/`setval` → int8), so a mixed projection's
    // ordinary columns are described exactly as anywhere else.
    let columns = expr_column_descs(row, ProjectionScope::empty().with_schema(schema));
    let mut values = Vec::with_capacity(row.len());
    for expr in row {
        let value = if let Some((func, seq, args)) = expr.as_sequence_fn() {
            eval_sequence_fn(func, seq, args, schema, session, env)?
        } else if let Some(args) = expr.as_advisory_lock() {
            // The arguments evaluate as any function's do, and the call is strict.
            let mut values = Vec::with_capacity(args.len());
            for arg in args {
                values.push(eval_expr_subquery(
                    arg,
                    &[],
                    &[],
                    &[],
                    ScopeIdentity::Anonymous,
                    &[],
                    schema,
                    storage,
                    env,
                )?);
            }
            if values.iter().any(Value::is_null) {
                Value::Null
            } else {
                super::expr::eval_advisory_xact_lock(&values)?
            }
        } else {
            // An ordinary projected expression: evaluate it normally. A *nested*
            // sequence function or advisory lock inside it fails loud there.
            eval_expr_subquery(
                expr,
                &[],
                &[],
                &[],
                ScopeIdentity::Anonymous,
                &[],
                schema,
                storage,
                env,
            )?
        };
        values.push(value);
    }
    Ok(QueryResult {
        shape: ResultShape::Columns(columns),
        rows: vec![values],
        command_tag: "SELECT 1".to_string(),
        notices: Vec::new(),
        tx_command: None,
        swapped_sequences: Vec::new(),
    })
}

/// Evaluate one sequence function against `schema.sequences` + the session cache.
///
/// This is the single place sequence state is mutated by a projection; it is also
/// reused by the `INSERT … VALUES(nextval(...))` path. `args` are already validated
/// for arity at build time (setval carries `[n]` or `[n, is_called]`; the others
/// none).
///
/// `setval` is **STRICT**: a NULL value or `is_called` argument yields NULL rather
/// than an error, and does so ahead of the kind gate — `setval('<a table>', NULL)` is
/// NULL on 18.6, not `42809`. Strictness is reached only by a call the **overload**
/// resolution accepted, though, so it is not a property of NULL as such: a NULL whose
/// declared type no `setval` takes (`NULL::text`) is `42883` and never gets there (see
/// [`check_setval_overload`]). (A NULL *name* is out of reach here: pglet's parser
/// requires a string literal, so `setval(NULL, 5)` — NULL in Postgres — fails loud
/// `0A000` before this.)
///
/// # Errors
///
/// A name **no relation at all** holds → `42P01`; a name held by a **table or an
/// index** → `42809` (Postgres's `validate_relation_kind`, reached from
/// `init_sequence` by all three functions — measured on 18.6 for `nextval`, `currval`
/// and `setval` alike); undefined `currval` → `55000`; a malformed setval argument →
/// a type error.
pub(super) fn eval_sequence_fn(
    func: SequenceFunc,
    seq: &str,
    args: &[Expr],
    schema: &mut Schema,
    session: &mut SequenceSession,
    env: EvalEnv<'_>,
) -> Result<Value> {
    // Postgres resolves the name, evaluates the arguments, applies **strictness**, and
    // only then runs the body — four steps, and a single up-front gate gets three of the
    // four orders wrong. `'x'::regclass` is coerced during parse analysis, ahead of
    // everything: `setval('<no such relation>', 1/0)` is `42P01`, not `22012`. The
    // arguments are then evaluated as ordinary function arguments. `setval` is
    // **STRICT**, so a NULL among them short-circuits the call and the result is NULL —
    // ahead of the body, which is where `init_sequence`'s `validate_relation_kind`
    // raises `42809`. So `setval('<a table>', 1/0)` is `22012`, `setval('<a table>',
    // NULL)` is **NULL**, and `setval('<a table>', 3)` is `42809`. All measured on 18.6;
    // an eighth scale of the error-order contract, and the one place *consolidating* a
    // check moved an error rather than adding one. (The *argument* error itself can still
    // diverge on its own terms: a malformed literal is `22P02` there and `42804` here,
    // because pglet's `setval` takes the value's parsed type rather than coercing an
    // unknown literal to `bigint`. That is a separate gap, and this ordering is what
    // makes it reachable at all.)
    //
    // Ahead of all four, though, is a step that is not evaluation at all: Postgres
    // resolves the **overload** from the argument *types* during parse analysis, and
    // only then coerces the name literal to `regclass`. So a wrongly-typed argument
    // beats even the missing relation — `setval('<no such relation>', NULL::text)` is
    // `42883`, not `42P01` (measured on 18.6).
    if func == SequenceFunc::Setval {
        check_setval_overload(args, schema)?;
    }

    // `regclass` finds a relation of **any** kind, which is why this step and the kind
    // gate are separate: a table's name resolves here and fails at the gate.
    if schema.relation_kind(seq).is_none() {
        return Err(Error::SequenceNotFound(seq.to_string()));
    }
    match func {
        SequenceFunc::Nextval => {
            ensure_sequence_relation(schema, seq)?;
            let sequence = schema
                .sequences
                .get_mut(seq)
                .ok_or_else(|| Error::SequenceNotFound(seq.to_string()))?;
            sequence.last_value += 1;
            let value = sequence.last_value;
            session.record_currval(seq, value);
            Ok(Value::Int8(value))
        }
        SequenceFunc::Currval => {
            // The relation must be a sequence (else `42809`), then have been advanced
            // this session (else `55000`).
            ensure_sequence_relation(schema, seq)?;
            let value = session
                .currval(seq)
                .ok_or_else(|| Error::CurrvalNotDefined(seq.to_string()))?;
            Ok(Value::Int8(value))
        }
        SequenceFunc::Setval => {
            // The overload gate above has already run. Evaluate the value and optional
            // `is_called` flag next (literals / bound params — no schema/storage
            // needed), **then** strictness, **then** the kind gate, then apply: an
            // argument error outranks the NULL short-circuit (a STRICT function's
            // arguments are all evaluated before the call is skipped, so
            // `setval(s, 1/0, NULL)` is `22012` on both engines), and both outrank the
            // `42809` (see above). The mirrored spelling `setval(s, NULL, 1/0)` is
            // **not** the same test — `1/0` is `integer` and there is no
            // `setval(regclass, bigint, integer)`, so it never reaches evaluation and
            // the overload gate answers `42883`.
            let value = eval_expr_env(&args[0], env)?;
            let flag = args
                .get(1)
                .map(|flag| eval_expr_env(flag, env))
                .transpose()?;
            if value.is_null() || flag.as_ref().is_some_and(Value::is_null) {
                return Ok(Value::Null);
            }
            let n = value_to_i64(&value)?;
            let is_called = flag.as_ref().map_or(Ok(true), value_to_bool)?;
            ensure_sequence_relation(schema, seq)?;
            let Some(sequence) = schema.sequences.get_mut(seq) else {
                return Err(Error::SequenceNotFound(seq.to_string()));
            };
            // pglet models a sequence as its last-issued counter, so `nextval` does
            // `last_value += 1; return last_value`. is_called=true: last_value=n (next
            // nextval → n+1) and currval reports n. is_called=false: last_value=n-1
            // (next nextval returns n exactly) and currval is left undefined — *left*,
            // not cleared: a prior `currval` in this session keeps its value, as it
            // does in Postgres.
            //
            // Only the counter moves. Writing a whole fresh record here would erase the
            // sequence's `start` (so a later bare `ALTER SEQUENCE … RESTART` would go
            // to the wrong place) and its `owner` (so the dump would stop emitting
            // `OWNED BY`) — both invisible to any query that only reads `nextval`.
            sequence.last_value = if is_called { n } else { n.wrapping_sub(1) };
            if is_called {
                session.record_currval(seq, n);
            }
            Ok(Value::Int8(n))
        }
    }
}

/// Resolve `setval`'s overload from its argument **types**, ahead of any evaluation.
///
/// Postgres registers exactly two: `setval(regclass, bigint)` and `setval(regclass,
/// bigint, boolean)`. Function resolution runs during parse analysis over the argument
/// types, so an argument no candidate accepts is `42883` *before* the name's `regclass`
/// coercion, before the arguments are evaluated, before strictness, and before the
/// relkind gate — all four measured on 18.6, and all four the wrong answer if the
/// value-level checks are left to speak alone. A value-level check structurally cannot
/// speak for a NULL: `Value::Null` carries no type, so a `NULL::text` reaches the STRICT
/// short-circuit and *succeeds* where Postgres refuses the call outright.
///
/// The accepted types are the ones with an **implicit** cast to the parameter:
/// `int2`/`int4`/`int8` for the value (`numeric` and `float8` are assignment-only, so
/// `setval(s, 5.0)` is `42883` too) and `bool` for `is_called` (`integer` is *not*
/// implicitly boolean, which is why `setval(s, 5, 1)` is `42883` where `setval(s, 5,
/// 'true')` **passes this gate** — it is an unknown literal, per the next paragraph,
/// and pglet then rejects it one step later).
///
/// An argument of **unknown** type is accepted, as Postgres accepts it: a bare literal
/// (`'5'`, `NULL`) stays `unknown` through resolution and coerces to the parameter type
/// afterwards. pglet types a bare string literal as `text` — it has no `unknown` — so
/// the shape is read here rather than the inferred type, keeping `setval(s, '5')` out
/// of this gate. Getting *past* the gate is not the same as being accepted, and the
/// literal-coercion gap that follows applies to **both** literal-taking arguments:
/// `setval(s, '5')` is `42804` where Postgres coerces and answers `5`, and so is
/// `setval(s, 5, 'true')` (`value_to_bool` refuses the text exactly as `value_to_i64`
/// refuses it — measured, both recorded in `EDGES.md`).
/// Deliberately not widened into a `42883` here, which would be the wrong error for a
/// call Postgres resolves. A `$N` parameter is likewise unknown until bind.
///
/// # Errors
///
/// `Error::UndefinedFunction` (`42883`) naming the argument types, as Postgres does.
fn check_setval_overload(args: &[Expr], schema: &Schema) -> Result<()> {
    let value_type = static_arg_type(&args[0], schema);
    let flag_type = args.get(1).and_then(|flag| static_arg_type(flag, schema));
    let value_ok =
        value_type.is_none_or(|t| matches!(t, TypeOid::Int2 | TypeOid::Int4 | TypeOid::Int8));
    let flag_ok = flag_type.is_none_or(|t| t == TypeOid::Bool);
    if value_ok && flag_ok {
        return Ok(());
    }
    let mut signature = format!("unknown, {}", opt_type_oid_display_name(value_type));
    if args.len() > 1 {
        signature.push_str(", ");
        signature.push_str(opt_type_oid_display_name(flag_type));
    }
    Err(Error::UndefinedFunction(format!(
        "function setval({signature}) does not exist"
    )))
}

/// The argument's type as **function resolution** sees it: `None` for Postgres's
/// `unknown` — a bare string literal, a bare `NULL`, or a `$N` parameter — and the
/// inferred type otherwise (a cast, a column, an operator result).
fn static_arg_type(expr: &Expr, schema: &Schema) -> Option<TypeOid> {
    match expr {
        // A bare literal is `unknown` in Postgres and `text` in pglet's IR; reading
        // `infer_type` here would reject `setval(s, '5')`, which Postgres accepts.
        Expr::Literal(Literal::String(_)) => None,
        other => other.infer_type(&PlannerScope::none(schema)),
    }
}

/// Coerce a `setval` value argument to `i64` (the sequence counter type).
fn value_to_i64(value: &Value) -> Result<i64> {
    match value {
        Value::Int2(v) => Ok(i64::from(*v)),
        Value::Int4(v) => Ok(i64::from(*v)),
        Value::Int8(v) => Ok(*v),
        other => Err(Error::Type(format!(
            "setval() requires an integer value, got {:?}",
            other.type_oid()
        ))),
    }
}

/// Coerce a `setval` `is_called` argument to `bool`.
fn value_to_bool(value: &Value) -> Result<bool> {
    match value {
        Value::Bool(b) => Ok(*b),
        other => Err(Error::Type(format!(
            "setval() is_called must be boolean, got {:?}",
            other.type_oid()
        ))),
    }
}
