//! The advisory-lock functions, answered against the session's holds.
//!
//! Every key is **the writer** (`EDGES.md` row 250): a locking call answers only where
//! the host took the writer before the statement ran, which is a top-level item of the
//! statement's own projection ([`Plan::advisory_projection_items`]). A
//! session-level call also counts a hold of its key here, in the session's
//! [`AdvisoryLocks`], and the host keeps the writer while any is held — past the block,
//! to the matching unlocks or the session's end, as 18.6 holds a session-level lock.
//!
//! The projection is evaluated once per row it produces, in row order, after its input
//! is produced whole, so a call over `n` rows takes or gives back `n` holds, as 18.6's
//! does — which is why a position whose rows a later node limits, deduplicates, groups or
//! combines is not one the call answers in.

use crate::Result;
use crate::catalog::Schema;
use crate::planner::{AdvisoryFunc, Expr, Plan};
use crate::types::{ResultShape, Value};

use super::Storage;
use super::expr::{EvalEnv, no_such_overload};
use super::types::{Notice, QueryResult};

/// A session-level advisory lock's key — 18.6's two lock spaces.
///
/// One `bigint`, and a pair of `int4`s: `pg_advisory_lock(1)` and `pg_advisory_lock(0, 1)`
/// name different locks there (the locktag's `objsubid` tells the two forms apart), and
/// here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdvisoryKey {
    /// `pg_advisory_lock(bigint)`.
    Int8(i64),
    /// `pg_advisory_lock(int4, int4)`.
    Pair(i32, i32),
}

/// The session-level advisory locks one session holds, each key with its count.
///
/// 18.6 stacks a hold per call and releases one per unlock (measured: two locks of a key
/// need two unlocks; a third answers `false` with its `WARNING`).
///
/// One per session: on the native server every connection's
/// ([`super::SequenceSession::advisory`]), and on the embedded `Database` the one logical
/// connection holding the writer's. A transaction-level hold
/// (`pg_advisory_xact_lock`) is not counted here: it is the writer alone, released where
/// the block ends, and no unlock gives it back — on 18.6 either.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AdvisoryLocks {
    held: Vec<(AdvisoryKey, u64)>,
}

impl AdvisoryLocks {
    /// Whether the session holds no session-level lock — when a host may release the
    /// writer it holds for them.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.held.is_empty()
    }

    /// One more hold of `key`.
    pub fn lock(&mut self, key: AdvisoryKey) {
        match self.held.iter_mut().find(|(k, _)| *k == key) {
            Some((_, count)) => *count += 1,
            None => self.held.push((key, 1)),
        }
    }

    /// One hold of `key` given back — `false` when the session holds none.
    pub fn unlock(&mut self, key: AdvisoryKey) -> bool {
        let Some(at) = self.held.iter().position(|(k, _)| *k == key) else {
            return false;
        };
        let count = &mut self.held[at].1;
        *count -= 1;
        if *count == 0 {
            self.held.remove(at);
        }
        true
    }

    /// Every hold given back — `pg_advisory_unlock_all()`, `DISCARD ALL`, and a session's
    /// end.
    pub fn unlock_all(&mut self) {
        self.held.clear();
    }
}

/// The key a call over non-NULL `args` names, after 18.6's overload check — `None` for
/// `pg_advisory_unlock_all()`, which names none.
///
/// `(bigint)` takes an `int2` / `int4` / `int8`; `(int4, int4)` takes an `int2` / `int4`
/// in each place and refuses an `int8` (`42883`), as 18.6 does (measured:
/// `pg_advisory_lock(1::int, 2::int8)`). A string argument is refused where 18.6 takes an
/// unknown literal (`pg_advisory_lock('1')`), the `col_description` reason (`EDGES.md`
/// row 250).
///
/// # Errors
///
/// `42883` for an argument list no overload takes.
pub fn advisory_key(func: AdvisoryFunc, args: &[Value]) -> Result<Option<AdvisoryKey>> {
    let small = |v: &Value| match v {
        Value::Int2(n) => Some(i32::from(*n)),
        Value::Int4(n) => Some(*n),
        _ => None,
    };
    let key = match (func, args) {
        (AdvisoryFunc::UnlockAll, []) => return Ok(None),
        (AdvisoryFunc::UnlockAll, _) => None,
        (_, [Value::Int2(n)]) => Some(AdvisoryKey::Int8(i64::from(*n))),
        (_, [Value::Int4(n)]) => Some(AdvisoryKey::Int8(i64::from(*n))),
        (_, [Value::Int8(n)]) => Some(AdvisoryKey::Int8(*n)),
        (_, [a, b]) => small(a).zip(small(b)).map(|(a, b)| AdvisoryKey::Pair(a, b)),
        _ => None,
    };
    key.map(Some)
        .ok_or_else(|| no_such_overload(func.name(), args))
}

/// The refusal of a non-NULL call evaluated anywhere but a top-level item of the
/// statement's own projection — where no host took the writer and nothing holds the
/// session's locks.
#[must_use]
pub fn off_projection(func: AdvisoryFunc) -> crate::Error {
    crate::Error::NotImplemented(format!(
        "{} is supported only as a top-level item of the statement's own SELECT list, with no LIMIT, DISTINCT, grouping, window or set operation",
        func.name()
    ))
}

/// One advisory call over its evaluated `args`, against the session's `locks`.
///
/// Strict: a NULL argument answers NULL and touches nothing (18.6 takes no lock for it;
/// here a locking call's statement still took the writer). A `pg_advisory_unlock` of a key
/// the session holds no session-level lock of answers `false` and adds 18.6's `WARNING`
/// to `notices`.
///
/// # Errors
///
/// `42883` for an argument list no overload takes.
pub fn eval_advisory_call(
    func: AdvisoryFunc,
    args: &[Value],
    locks: &mut AdvisoryLocks,
    notices: &mut Vec<Notice>,
) -> Result<Value> {
    if args.iter().any(Value::is_null) {
        return Ok(Value::Null);
    }
    let key = advisory_key(func, args)?;
    // `void` is the empty text a `TypeOid::Void` descriptor carries.
    let void = Value::Text(String::new());
    Ok(match (func, key) {
        (AdvisoryFunc::XactLock, _) => void,
        (AdvisoryFunc::Lock, Some(key)) => {
            locks.lock(key);
            void
        }
        (AdvisoryFunc::Unlock, Some(key)) => {
            let held = locks.unlock(key);
            if !held {
                notices.push(Notice::warning(
                    "you don't own a lock of type ExclusiveLock".into(),
                ));
            }
            Value::Bool(held)
        }
        (AdvisoryFunc::UnlockAll, _) => {
            locks.unlock_all();
            void
        }
        (AdvisoryFunc::Lock | AdvisoryFunc::Unlock, None) => {
            return Err(no_such_overload(func.name(), args));
        }
    })
}

/// Execute a `Project` root whose items hold a top-level advisory call
/// ([`Plan::advisory_projection_items`]): its input's rows, produced whole, then each
/// row's items in order — the advisory calls against `locks`, every other item as the
/// ordinary projection evaluates it.
///
/// # Errors
///
/// The input's, an item's, or a call's own.
pub(super) fn execute_advisory_project<S: Storage>(
    plan: &Plan,
    schema: &Schema,
    storage: &S,
    locks: &mut AdvisoryLocks,
    env: EvalEnv<'_>,
) -> Result<QueryResult> {
    let mut notices = Vec::new();
    let (rows, columns) = super::query::execute_project_with(
        plan,
        schema,
        storage,
        env,
        &mut |expr, eval| match expr.as_advisory_call() {
            Some((func, args)) => {
                let values = args.iter().map(&mut *eval).collect::<Result<Vec<_>>>()?;
                eval_advisory_call(func, &values, locks, &mut notices)
            }
            None => eval(expr),
        },
    )?;
    let row_count = rows.len();
    Ok(QueryResult {
        shape: ResultShape::Columns(columns),
        rows,
        command_tag: format!("SELECT {row_count}"),
        notices,
        tx_command: None,
        swapped_sequences: Vec::new(),
    })
}

/// Whether `expr` is a top-level advisory call that takes a lock
/// ([`AdvisoryFunc::locks`]).
pub fn is_locking_call(expr: &Expr) -> bool {
    expr.as_advisory_call()
        .is_some_and(|(func, _)| func.locks())
}

#[cfg(test)]
mod tests {
    use super::{AdvisoryKey, AdvisoryLocks, advisory_key, eval_advisory_call};
    use crate::planner::AdvisoryFunc;
    use crate::types::Value;

    #[test]
    fn holds_stack_and_unlock_one_at_a_time() {
        let mut locks = AdvisoryLocks::default();
        let mut notices = Vec::new();
        let key = [Value::Int8(3_853_314_791_062_309_107)];
        for _ in 0..2 {
            eval_advisory_call(AdvisoryFunc::Lock, &key, &mut locks, &mut notices).unwrap();
        }
        let unlock = |locks: &mut AdvisoryLocks, notices: &mut Vec<_>| {
            eval_advisory_call(AdvisoryFunc::Unlock, &key, locks, notices).unwrap()
        };
        assert_eq!(unlock(&mut locks, &mut notices), Value::Bool(true));
        assert!(!locks.is_empty());
        assert_eq!(unlock(&mut locks, &mut notices), Value::Bool(true));
        assert!(locks.is_empty());
        assert!(notices.is_empty());
        assert_eq!(unlock(&mut locks, &mut notices), Value::Bool(false));
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0].sqlstate, "01000");
        assert_eq!(
            notices[0].message,
            "you don't own a lock of type ExclusiveLock"
        );
    }

    #[test]
    fn the_two_key_spaces_are_distinct_and_unlock_all_clears_both() {
        let mut locks = AdvisoryLocks::default();
        locks.lock(AdvisoryKey::Int8(1));
        locks.lock(AdvisoryKey::Pair(0, 1));
        assert!(locks.unlock(AdvisoryKey::Int8(1)));
        assert!(!locks.unlock(AdvisoryKey::Int8(1)));
        assert!(!locks.is_empty());
        locks.lock(AdvisoryKey::Int8(1));
        locks.unlock_all();
        assert!(locks.is_empty());
        assert!(!locks.unlock(AdvisoryKey::Pair(0, 1)));
    }

    #[test]
    fn a_null_argument_is_null_and_takes_nothing_and_a_mistyped_one_is_42883() {
        let mut locks = AdvisoryLocks::default();
        let mut notices = Vec::new();
        let null = eval_advisory_call(AdvisoryFunc::Lock, &[Value::Null], &mut locks, &mut notices);
        assert_eq!(null.unwrap(), Value::Null);
        assert!(locks.is_empty());
        let unlock = eval_advisory_call(
            AdvisoryFunc::Unlock,
            &[Value::Null],
            &mut locks,
            &mut notices,
        );
        assert_eq!(unlock.unwrap(), Value::Null);
        assert!(notices.is_empty(), "a NULL unlock warns of nothing");
        for args in [
            vec![Value::Int4(1), Value::Int8(2)],
            vec![Value::Text("1".into())],
            vec![Value::Bool(true)],
        ] {
            let err = advisory_key(AdvisoryFunc::Lock, &args).unwrap_err();
            assert_eq!(err.sqlstate(), "42883", "{args:?}");
        }
        assert_eq!(
            advisory_key(AdvisoryFunc::Unlock, &[Value::Int2(1), Value::Int4(2)]).unwrap(),
            Some(AdvisoryKey::Pair(1, 2))
        );
    }
}
