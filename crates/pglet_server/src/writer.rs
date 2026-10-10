//! **The writer**: the one connection whose block may stage writes, held from the
//! block's first write to its end, and by an auto-commit write for its statement.
//!
//! The engine keeps one catalog and one store that every connection reads and writes in
//! place, and a block's writes stage against the state its snapshot holds. Two blocks
//! staging writes at once decided each against a state the other was changing: a key
//! committed under a primary key twice, a dangling foreign key, an update lost, and a
//! rollback that could not tell its own catalog changes from another connection's. So a
//! block takes the writer before it stages anything, and a second writer **waits** —
//! bounded by its session's `lock_timeout`, answered `55P03` past it — rather than stage
//! against a write it cannot see. A read takes it only to lock rows (a `SELECT nextval(…)`
//! or `setval(…)` is a write here). A statement's plan
//! (and a `COPY`'s column list) that fails because of the holder's uncommitted catalog
//! waits for the holder too (`Connection::plan_failure_is_the_writers`); what a plan that succeeds reads
//! of that catalog, and an error raised as it runs, are not covered (the repo's `EDGES.md`
//! rows 221, 238 and 240).
//!
//! One waitable resource: a waiter holds no other lock while it waits (the statement's
//! schema and storage guards and its read snapshot are dropped first), and the holder
//! never waits on the writer again, so no wait can close a cycle inside the server. A
//! later waitable resource must be taken strictly inside the writer, or that stops being
//! true.

use std::sync::Arc;
use std::time::{Duration, Instant};

use parking_lot::{Condvar, Mutex};
use pglet_sql::catalog::Schema;

/// How long a waiter sleeps between looks at the store: a waiter wakes when the writer
/// is released, and also this often to see whether the store it waits on has failed.
const POLL: Duration = Duration::from_millis(50);

/// The writer of one [`crate::connection::SharedState`].
#[derive(Debug, Default)]
pub struct WriterLock {
    state: Mutex<WriterState>,
    released: Condvar,
}

#[derive(Debug, Default)]
struct WriterState {
    /// The connection holding the writer.
    holder: Option<i32>,
    /// What the holder found when it took the writer ([`WriterLock::note_grant`]) —
    /// `None` until it says, which it does under the schema lock before it can change
    /// anything.
    grant: Option<Grant>,
    /// Set once the state this writer belongs to is replaced after its store failed:
    /// every waiter is woken and answered with the reason.
    failed: Option<String>,
}

/// What a holder of the writer found when it took it.
#[derive(Debug)]
struct Grant {
    /// The catalog generation: a catalog at any other holds a change the holder made
    /// and has not ended — or, for an auto-commit holder, one it has already committed.
    generation: u64,
    /// The catalog committed at the grant, held for a **block** — which leaves its
    /// changes in the shared catalog until it ends — so another connection can ask
    /// whether its failing plan fails there too. `None` for an auto-commit statement,
    /// whose changes are committed before the schema lock another connection plans
    /// under is released.
    committed: Option<Arc<Schema>>,
}

/// The instant a wait of `timeout_ms` milliseconds begun now gives up — `None` for `0`,
/// which waits without a bound, as 18.6's `lock_timeout = 0` does.
#[must_use]
pub fn deadline_after(timeout_ms: u64) -> Option<Instant> {
    (timeout_ms > 0).then(|| Instant::now() + Duration::from_millis(timeout_ms))
}

impl WriterLock {
    /// Take the writer for `conn_id`, waiting while another connection holds it — until
    /// `deadline`, or without a bound for `None` — and answering at once when `conn_id`
    /// already holds it.
    ///
    /// `store_failed` is asked each time the wait wakes: a waiter on a store that failed a
    /// durable write is answered with the failure rather than left waiting on a holder
    /// that may never send another statement.
    ///
    /// # Errors
    ///
    /// [`pglet_sql::Error::LockTimeout`] (`55P03`) past the deadline, or
    /// [`pglet_sql::Error::DatabaseMustBeReopened`] (`57P02`) when the store failed or its
    /// state was replaced ([`Self::fail_waiters`]).
    pub fn acquire(
        &self,
        conn_id: i32,
        deadline: Option<Instant>,
        store_failed: impl Fn() -> Option<String>,
    ) -> Result<(), pglet_sql::Error> {
        self.wait(conn_id, deadline, store_failed, true)
    }

    /// Wait, as [`Self::acquire`] does, until no **other** connection holds the writer,
    /// without taking it — for a statement that must see the catalog another
    /// connection's open block is changing before it can say what it is.
    ///
    /// # Errors
    ///
    /// As [`Self::acquire`].
    pub fn wait_for_release(
        &self,
        conn_id: i32,
        deadline: Option<Instant>,
        store_failed: impl Fn() -> Option<String>,
    ) -> Result<(), pglet_sql::Error> {
        self.wait(conn_id, deadline, store_failed, false)
    }

    /// [`Self::acquire`] (`take`) and [`Self::wait_for_release`]'s one loop.
    fn wait(
        &self,
        conn_id: i32,
        deadline: Option<Instant>,
        store_failed: impl Fn() -> Option<String>,
        take: bool,
    ) -> Result<(), pglet_sql::Error> {
        let mut state = self.state.lock();
        loop {
            if let Some(reason) = state.failed.clone().or_else(&store_failed) {
                return Err(pglet_sql::Error::DatabaseMustBeReopened(reason));
            }
            match state.holder {
                None => {
                    if take {
                        state.holder = Some(conn_id);
                        state.grant = None;
                    }
                    return Ok(());
                }
                Some(holder) if holder == conn_id => return Ok(()),
                Some(_) => {}
            }
            let slice = match deadline {
                Some(deadline) => {
                    let now = Instant::now();
                    if now >= deadline {
                        return Err(pglet_sql::Error::LockTimeout);
                    }
                    (deadline - now).min(POLL)
                }
                None => POLL,
            };
            self.released.wait_for(&mut state, slice);
        }
    }

    /// Record what `conn_id` found on taking the writer — the catalog `generation`, and
    /// for a block the catalog `committed` then — read under the schema lock before it
    /// changed anything. A no-op unless `conn_id` holds it and has not said yet.
    pub fn note_grant(&self, conn_id: i32, generation: u64, committed: Option<Arc<Schema>>) {
        let mut state = self.state.lock();
        if state.holder == Some(conn_id) && state.grant.is_none() {
            state.grant = Some(Grant {
                generation,
                committed,
            });
        }
    }

    /// The catalog committed when another connection's **block** took the writer, where
    /// the catalog now at `generation` holds a change that block made and has not ended
    /// — what `conn_id`'s plan failing at `generation` is planned against again, to ask
    /// whether that change caused the failure. `None` when `conn_id` holds the writer or
    /// no one does, when the holder is an auto-commit statement, when it has not said what
    /// it found (it cannot have changed anything yet: that takes the schema lock a planning
    /// connection holds), and when the catalog is still at the generation it found.
    pub fn foreign_grant_catalog(&self, conn_id: i32, generation: u64) -> Option<Arc<Schema>> {
        let state = self.state.lock();
        if state.holder.is_none_or(|holder| holder == conn_id) {
            return None;
        }
        state
            .grant
            .as_ref()
            .filter(|grant| grant.generation != generation)
            .and_then(|grant| grant.committed.clone())
    }

    /// Release the writer if `conn_id` holds it, waking the waiters. A no-op otherwise.
    pub fn release(&self, conn_id: i32) {
        let mut state = self.state.lock();
        if state.holder == Some(conn_id) {
            state.holder = None;
            state.grant = None;
            drop(state);
            self.released.notify_all();
        }
    }

    /// Whether `conn_id` holds the writer.
    pub fn is_held_by(&self, conn_id: i32) -> bool {
        self.state.lock().holder == Some(conn_id)
    }

    /// Wake every waiter and answer it `57P02` with `reason` — the state this writer
    /// belongs to has been replaced after its store failed a durable write
    /// (`crate::server::Server::connection_state`), so nothing it could wait for will
    /// be served from it again.
    pub fn fail_waiters(&self, reason: &str) {
        self.state.lock().failed = Some(reason.to_string());
        self.released.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use pglet_sql::catalog::Schema;

    use super::{WriterLock, deadline_after};

    #[test]
    fn a_second_writer_waits_for_the_release() {
        let lock = Arc::new(WriterLock::default());
        lock.acquire(1, deadline_after(1000), || None)
            .expect("free");
        let waiter = {
            let lock = Arc::clone(&lock);
            std::thread::spawn(move || {
                let started = Instant::now();
                lock.acquire(2, deadline_after(0), || None)
                    .expect("released");
                started.elapsed()
            })
        };
        std::thread::sleep(Duration::from_millis(150));
        assert!(lock.is_held_by(1));
        lock.release(1);
        let waited = waiter.join().expect("waiter");
        assert!(waited >= Duration::from_millis(100), "{waited:?}");
        assert!(lock.is_held_by(2));
    }

    #[test]
    fn a_holder_takes_it_again_at_once_and_a_bounded_wait_times_out_55p03() {
        let lock = WriterLock::default();
        lock.acquire(1, deadline_after(1000), || None)
            .expect("free");
        lock.acquire(1, deadline_after(1000), || None)
            .expect("its own");
        let started = Instant::now();
        let err = lock
            .acquire(2, deadline_after(120), || None)
            .expect_err("held");
        assert_eq!(err.sqlstate(), "55P03", "{err}");
        assert_eq!(err.to_string(), "canceling statement due to lock timeout");
        assert!(started.elapsed() >= Duration::from_millis(120));
        // Releasing a writer another connection holds does nothing.
        lock.release(2);
        assert!(lock.is_held_by(1));
    }

    /// The waits are bounded (two seconds) only so a broken wake fails as a `55P03`
    /// rather than hanging the suite; either wake answers well inside the bound.
    #[test]
    fn a_failed_state_wakes_its_waiters_with_57p02() {
        let lock = Arc::new(WriterLock::default());
        lock.acquire(1, deadline_after(0), || None).expect("free");
        let waiter = {
            let lock = Arc::clone(&lock);
            std::thread::spawn(move || lock.acquire(2, deadline_after(2000), || None))
        };
        std::thread::sleep(Duration::from_millis(100));
        lock.fail_waiters("disk gone");
        let err = waiter.join().expect("waiter").expect_err("failed");
        assert_eq!(err.sqlstate(), "57P02", "{err}");
        // And a store failure seen while waiting answers the same, with no wake-up.
        let other = WriterLock::default();
        other.acquire(1, deadline_after(0), || None).expect("free");
        let err = other
            .acquire(2, deadline_after(2000), || Some("fsync refused".into()))
            .expect_err("store failed");
        assert_eq!(err.sqlstate(), "57P02", "{err}");
    }

    /// Another connection's failing plan is planned again against a holder's committed
    /// catalog only when that holder is a block which said what it found and whose
    /// catalog has moved since. A holder that has taken the writer and not yet said —
    /// the window in which it waits for the schema lock the planning connection holds —
    /// has changed nothing, and an auto-commit holder's changes are committed, so both
    /// answer `None` and the plan's failure is the statement's own.
    #[test]
    fn a_grant_catalog_is_offered_only_for_a_blocks_moved_catalog() {
        let lock = WriterLock::default();
        assert!(lock.foreign_grant_catalog(2, 7).is_none(), "no holder");
        lock.acquire(1, deadline_after(0), || None).expect("free");
        for generation in [0, 5, 6, u64::MAX] {
            assert!(
                lock.foreign_grant_catalog(2, generation).is_none(),
                "taken, not yet noted: generation {generation}"
            );
        }
        lock.note_grant(1, 5, Some(Arc::new(Schema::default())));
        assert!(lock.foreign_grant_catalog(2, 5).is_none(), "not moved");
        assert!(lock.foreign_grant_catalog(2, 6).is_some(), "moved");
        assert!(lock.foreign_grant_catalog(1, 6).is_none(), "its own");
        // A second note does not replace the first.
        lock.note_grant(1, 6, None);
        assert!(
            lock.foreign_grant_catalog(2, 6).is_some(),
            "the first note stands"
        );
        lock.release(1);
        lock.acquire(3, deadline_after(0), || None).expect("free");
        lock.note_grant(3, 5, None);
        assert!(
            lock.foreign_grant_catalog(2, 6).is_none(),
            "an auto-commit holder"
        );
    }
}
