//! Connection state and handling.
//!
//! Engine assembly (file/btree/memory backends, double-meta recovery, schema restore),
//! the `StorageBackend` carrier, the executor adapters, and the commit semantics
//! all live in the `pglet` facade crate. This module owns only the server's
//! concurrency layer (`parking_lot` locks, lock ordering, MVCC snapshot capture),
//! per-connection session state, and the socket loop.

use std::collections::HashMap;
use std::io::{BufWriter, Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::Instant;

use parking_lot::{Mutex, MutexGuard};
use pglet::{Database, StorageBackend, TransactionSession};
use pglet_sql::catalog::{Schema, SessionSettings};
use pglet_sql::depth::CatalogDepth;
use pglet_sql::executor::{SequenceSession, TransactionCommand};
use pglet_sql::planner::CopyOptions;
use pglet_storage::btree::{BtreeSnapshot, WriteFailure};
use pglet_wire::{
    FrontendMessage, Session, SessionState, TransactionStatus, decode_message, encode_message,
};

// The `StorageBackend` carrier and the executor adapters are owned by the facade
// (`pglet`) so the engine assembly is single-sourced; the server re-exports them
// for its handlers.
pub use pglet::{SnapshotAdapter, StorageAdapter};

use crate::handlers::handle_message;
use crate::writer::{WriterLock, deadline_after};

/// A prepared statement (result of Parse).
#[derive(Debug, Clone)]
pub struct PreparedStatement {
    pub query: String,
    /// Each parameter's type as `infer_param_types` settled it — the Parse message's
    /// declared OID where the client sent one, else the inferred one — the list
    /// `ParameterDescription` reports and `Bind` decodes by. `None` = not inferred.
    pub param_types: Vec<Option<pglet_sql::types::TypeOid>>,
    /// Each parameter's **enum target** by catalog key, as `infer_param_targets`
    /// settled it — `None` for a parameter whose position names no enum. Kept beside
    /// `param_types` rather than folded into it because an enum column's `TypeOid` is
    /// `Text` and the OID therefore cannot carry it; `Bind` spends it on the decoded
    /// value, which is where 18.6 runs `enum_in`.
    pub param_enums: Vec<Option<String>>,
    /// The Parse message's own OIDs, `None` where the client left a parameter for
    /// inference. Kept apart from `param_types` because a re-plan must be handed what
    /// the client **declared** and nothing inference added: the declared types are
    /// part of the statement's parse analysis
    /// (`pglet_sql::parser::parse_single_with_param_types`), the inferred ones are not.
    pub declared_param_types: Vec<Option<pglet_sql::types::TypeOid>>,
    /// The plan built once at Parse, shared (immutable) with Describe/Execute so
    /// they don't re-parse + re-plan. `None` where Parse built no plan: the session
    /// commands the executor handles by string (`SET`, `SHOW`, `DEALLOCATE` — the
    /// `runs_outside_the_planner` set), and an empty statement (empty text, a lone
    /// `;`, comment-only). A statement whose plan *errored* is not one of those
    /// — `Parse` returns the error before this record is inserted, so no record
    /// exists to hold a `None`. Reused only while `plan_gen` still matches the live
    /// `Schema::generation`; a DDL, or a rollback that takes one back, moves the
    /// generation and forces a re-plan.
    pub plan: Option<Arc<pglet_sql::planner::Plan>>,
    /// The `Schema::generation` the cached `plan` was built against.
    pub plan_gen: u64,
}

/// A portal (result of Bind) - a prepared statement with bound parameters.
#[derive(Debug, Clone)]
pub struct Portal {
    /// The prepared statement's SQL (the re-plan fallback if the cached plan is
    /// stale).
    pub query: String,
    /// Decoded bound parameter values, indexed by `$N - 1`; `None` is a SQL NULL.
    /// Supplied to execution via the `EvalEnv` (the plan is not mutated to bind).
    pub bound_values: Vec<Option<pglet_sql::types::Value>>,
    /// Result format codes from client.
    pub result_formats: Vec<pglet_wire::protocol::FormatCode>,
    /// The prepared statement's cached plan (Arc-shared from Bind) + the schema
    /// generation it was planned against. Reused at Execute when still current.
    pub plan: Option<Arc<pglet_sql::planner::Plan>>,
    pub plan_gen: u64,
    /// The prepared statement's `declared_param_types`, for a re-plan at Describe or
    /// Execute.
    pub declared_param_types: Vec<Option<pglet_sql::types::TypeOid>>,
}

/// Shared state across all connections.
///
/// Both schema and storage are protected by mutexes. To prevent ABBA deadlocks,
/// always acquire locks through `lock_both()` which enforces consistent ordering.
///
/// The engine itself (schema + storage assembly) is built by `pglet::Database`;
/// this type splits the assembled `Database` into separately locked parts so the
/// MVCC read path can drop the schema lock and read from a snapshot holding no
/// locks.
///
/// # Lock ordering
///
/// When both locks are needed, schema must be acquired before storage.
/// The `lock_both()` method enforces this automatically.
///
/// # MVCC Support
///
/// For B+tree storage, `read_snapshot()` provides lock-free reads:
/// - Brief lock to capture snapshot
/// - Lock released immediately
/// - Snapshot allows concurrent reads without holding the global lock
///
/// Memory storage falls back to `lock_both()` for all operations.
pub struct SharedState {
    schema: Mutex<Schema>,
    storage: Mutex<StorageBackend>,
    /// Whether unsupported SQL types coerce to `TEXT` instead of erroring
    /// (the `--coerce-unknown-types` flag). Default `false` (fail loud). Read
    /// once per connection thread to seed the per-thread resolver toggle.
    coerce_unknown_types: bool,
    /// The store's record of a write or `fsync` it could not complete, read without
    /// the storage lock at every message ([`Self::write_failure_reason`]). `None` for a
    /// backend that persists nothing.
    write_failure: Option<WriteFailure>,
    /// The writer: the one connection whose block may stage writes ([`WriterLock`]).
    /// Per state, so a state reopened after its store failed starts with it free.
    pub writer: WriterLock,
    /// The `lock_timeout` a connection starts with, in milliseconds (`0`: no bound) —
    /// the `--lock-timeout` flag, else pglet's default
    /// ([`pglet_sql::catalog::DEFAULT_LOCK_TIMEOUT_MS`]).
    default_lock_timeout_ms: u64,
}

impl SharedState {
    /// Split an assembled `Database` into separately locked schema + storage.
    pub fn from_database(db: Database) -> Self {
        let (schema, storage) = db.into_parts();
        let write_failure = storage.write_failure();
        Self {
            schema: Mutex::new(schema),
            storage: Mutex::new(storage),
            coerce_unknown_types: false,
            write_failure,
            writer: WriterLock::default(),
            default_lock_timeout_ms: pglet_sql::catalog::DEFAULT_LOCK_TIMEOUT_MS,
        }
    }

    /// The reason the store failed a durable write, once it has: from then on every
    /// connection served from this state is ended and no new one is, since the
    /// in-memory state may hold a commit the durable state does not. The server serves
    /// new connections from a state reopened from the durable slot instead
    /// ([`crate::server::Server::connection_state`]).
    #[must_use]
    pub fn write_failure_reason(&self) -> Option<&str> {
        self.write_failure.as_ref().and_then(WriteFailure::get)
    }

    /// Set whether unsupported SQL types coerce to `TEXT` instead of erroring.
    ///
    /// Builder-style: applied to the assembled state before it is shared across
    /// connection threads. Each connection thread reads this once (in
    /// `handle_connection`) to seed its per-thread resolver toggle.
    #[must_use]
    pub const fn with_coerce_unknown_types(mut self, coerce: bool) -> Self {
        self.coerce_unknown_types = coerce;
        self
    }

    /// Whether this server coerces unsupported types to `TEXT`.
    #[must_use]
    pub const fn coerce_unknown_types(&self) -> bool {
        self.coerce_unknown_types
    }

    /// Set the `lock_timeout` a connection starts with (`0`: no bound). Builder-style,
    /// like [`Self::with_coerce_unknown_types`].
    #[must_use]
    pub const fn with_default_lock_timeout(mut self, ms: u64) -> Self {
        self.default_lock_timeout_ms = ms;
        self
    }

    /// The `lock_timeout` a connection starts with, in milliseconds.
    #[must_use]
    pub const fn default_lock_timeout_ms(&self) -> u64 {
        self.default_lock_timeout_ms
    }

    /// Create shared state with in-memory storage.
    pub fn in_memory() -> Self {
        Self::from_database(Database::in_memory())
    }

    /// Create shared state with B+tree storage (`CoW` MVCC).
    ///
    /// B+tree provides CoW-based MVCC with single-writer semantics.
    /// Lower write amplification than LSM, natural snapshot isolation.
    pub fn with_btree() -> Self {
        Self::from_database(Database::with_btree())
    }

    /// Acquire both schema and storage locks in consistent order.
    ///
    /// This is the only way to access the shared state, ensuring deadlock-free
    /// lock acquisition (schema first, then storage).
    ///
    /// Returns tuple of (`schema_guard`, `storage_guard`).
    #[inline]
    pub fn lock_both(&self) -> (MutexGuard<'_, Schema>, MutexGuard<'_, StorageBackend>) {
        let schema = self.schema.lock();
        let storage = self.storage.lock();
        (schema, storage)
    }

    /// Acquire only the schema lock.
    ///
    /// Use this when you only need to read schema metadata (e.g., for planning)
    /// and will use a snapshot for data access.
    #[inline]
    pub fn lock_schema(&self) -> MutexGuard<'_, Schema> {
        self.schema.lock()
    }

    /// Acquire only the storage lock.
    ///
    /// Safe beside [`lock_both`](Self::lock_both) because no path holds the storage
    /// guard while acquiring schema: `lock_both` is the only two-lock site and takes
    /// them in that order, and [`read_snapshot`](Self::read_snapshot) releases storage
    /// before returning. So a storage-only acquisition cannot be one half of an ABBA.
    /// Use it where a path touches storage and nothing else — taking the schema lock it
    /// does not read only makes a waiter of a statement that had no business waiting.
    #[inline]
    pub fn lock_storage(&self) -> MutexGuard<'_, StorageBackend> {
        self.storage.lock()
    }

    // ========================================================================
    // MVCC Support (Lock-Free Reads)
    // ========================================================================

    /// Get a read snapshot for B+tree storage.
    ///
    /// This method briefly locks the storage mutex to capture a snapshot,
    /// then immediately releases the lock. The returned `BtreeSnapshot`
    /// can be used for lock-free reads.
    ///
    /// Returns `None` for memory storage (use `lock_both()` instead).
    pub fn read_snapshot(&self) -> Option<BtreeSnapshot> {
        let storage = self.storage.lock();
        storage.read_snapshot()
        // storage mutex released here
    }
}

/// An in-progress `COPY … FROM STDIN`.
///
/// Set when the server returns `CopyInResponse` (the `Connection` enters copy-in
/// mode) and consumed at `CopyDone`, where `execute_copy_from` ingests `buffer`.
/// While it is `Some`, [`handle_message`](crate::handlers::handle_message) routes
/// every inbound frame to the copy-in handler — the simple-query analogue of the
/// extended protocol's `skip_until_sync` flag. `options` is the resolved
/// [`Plan::Copy`](pglet_sql::planner::Plan::Copy) bundle, borrowed as the engine's
/// `CopyParams` to rebuild the ingest at `CopyDone`.
pub struct CopyInState {
    /// The resolved table / column-list / format / framing options (the shared
    /// [`CopyOptions`], borrowed as the engine's `CopyParams` at `CopyDone`).
    pub options: CopyOptions,
    /// Whether the COPY arrived over the **extended** protocol (tokio-postgres /
    /// the spine driver prepare the statement). Extended COPY defers its trailing
    /// `ReadyForQuery` to the client's `Sync`; the simple-query path (psql) appends
    /// it itself. Threaded so `CopyDone` resynchronizes the right way.
    pub extended: bool,
    /// The accumulating `CopyData` payload, ingested at `CopyDone`.
    pub buffer: Vec<u8>,
}

/// Connection state - wire protocol session + reference to shared state.
pub struct Connection {
    pub session: Session,
    /// Shared state (schema + storage).
    pub shared: Arc<SharedState>,
    /// Prepared statements by name (empty string = unnamed).
    pub prepared: HashMap<String, PreparedStatement>,
    /// Portals by name (empty string = unnamed).
    pub portals: HashMap<String, Portal>,
    /// The transaction block: status, rollback frames, and the exit-statement
    /// classifier. The type is the embed facade's ([`pglet::TransactionSession`]) —
    /// one state machine for the native server and the wasm bindings alike, since two
    /// copies had already drifted into a divergence about committed data.
    pub tx: TransactionSession,
    /// Extended-protocol error recovery: set once an extended-query message has
    /// errored, it makes the router discard every following message until the next
    /// `Sync` (which clears it and re-synchronizes). Postgres's
    /// skip-until-`Sync` behavior — without it a pipelined `Bind`/`Execute` after a
    /// failed step would emit a cascade of extra errors. See `handlers::handle_message`.
    pub skip_until_sync: bool,
    /// `COPY … FROM STDIN` sub-protocol state: `Some` once the server has sent
    /// `CopyInResponse` and is accumulating the inbound `CopyData` stream, until
    /// `CopyDone`/`CopyFail`. While set, the router forwards every inbound frame to
    /// the copy-in handler (mirroring the `skip_until_sync` mode pattern).
    pub copy_in: Option<CopyInState>,
    /// Per-connection state for the sequence functions (`currval`'s last-`nextval`
    /// cache). Kept on the connection — not in the shared `Schema` — because
    /// `currval` reports the last `nextval` in **this** session.
    pub sequence_session: SequenceSession,
    /// The runtime parameters whose value belongs to this session rather than to
    /// the build — `application_name`, seeded from the startup packet.
    ///
    /// Read twice per session command by `execute_query_core`: Postgres reports a
    /// `GUC_REPORT` parameter at connect and then **only when its value moves**, so
    /// the caller compares `catalog::startup_parameters` across the call rather than
    /// having the statement announce itself.
    pub settings: SessionSettings,
    /// The shared catalog's deepest stored expression, asked at the entry of every
    /// statement this connection runs (`pglet_sql::depth::check_catalog_walks`): the
    /// facade's check, at this host's statement runner (`execute::run_one`) and COPY
    /// sites. No route reaches a refusal here today — every connection thread is
    /// declared [`pglet_sql::depth::SERVER_THREAD_STACK`] at
    /// [`pglet_sql::depth::SERVER_MAX_DEPTH`], the bounds every stored expression was
    /// built or opened under, and a statement enters at the top of it — so the check
    /// holds the server to the rule the embed hosts need rather than guarding a case of
    /// its own.
    pub catalog_depth: CatalogDepth,
}

impl Connection {
    pub fn new(process_id: i32, shared: Arc<SharedState>) -> Self {
        // Replaced at `Startup` with the packet's own `application_name`, and seeded
        // there with the same default `lock_timeout`; a connection that never sends one
        // keeps Postgres's empty default name.
        let mut settings = SessionSettings::default();
        settings.set_default_lock_timeout(shared.default_lock_timeout_ms());
        Self {
            session: Session::new(process_id),
            shared,
            prepared: HashMap::new(),
            portals: HashMap::new(),
            tx: TransactionSession::new(),
            skip_until_sync: false,
            copy_in: None,
            sequence_session: SequenceSession::new(),
            settings,
            catalog_depth: CatalogDepth::default(),
        }
    }

    /// The transaction block's current state — what `ReadyForQuery` reports.
    #[must_use]
    pub const fn transaction_status(&self) -> TransactionStatus {
        self.tx.status()
    }

    /// Whether this connection holds the writer ([`WriterLock`]).
    #[must_use]
    pub fn holds_writer(&self) -> bool {
        self.shared.writer.is_held_by(self.session.process_id)
    }

    /// When a wait for the writer begun now gives up — this session's `lock_timeout`
    /// from now, or `None` for `0`. One statement takes one deadline, however many waits
    /// it makes.
    #[must_use]
    pub fn writer_deadline(&self) -> Option<Instant> {
        deadline_after(self.settings.lock_timeout_ms())
    }

    /// Take the writer, waiting while another connection holds it until `deadline` — the
    /// caller holds **no** schema or storage guard and no read snapshot, which is what
    /// keeps a wait from closing a cycle ([`WriterLock`]).
    ///
    /// # Errors
    ///
    /// `55P03` past the deadline, or `57P02` once this state's store has failed.
    pub fn acquire_writer(&self, deadline: Option<Instant>) -> Result<(), pglet_sql::Error> {
        let shared = &self.shared;
        shared
            .writer
            .acquire(self.session.process_id, deadline, || {
                shared.write_failure_reason().map(str::to_string)
            })
    }

    /// Wait, holding no guard, until no other connection holds the writer — for a
    /// statement whose plan failed because of another connection's uncommitted catalog
    /// change ([`Self::plan_failure_is_the_writers`]), which is planned again once that
    /// hold ends.
    ///
    /// # Errors
    ///
    /// As [`Self::acquire_writer`].
    pub fn wait_for_foreign_writer(
        &self,
        deadline: Option<Instant>,
    ) -> Result<(), pglet_sql::Error> {
        let shared = &self.shared;
        shared
            .writer
            .wait_for_release(self.session.process_id, deadline, || {
                shared.write_failure_reason().map(str::to_string)
            })
    }

    /// Whether planning's `error`, made against the catalog at `generation`, is another
    /// connection's uncommitted catalog change speaking rather than the statement's own
    /// answer — so the statement must wait for that connection's block to end and plan
    /// again. It is when that block holds the writer, has changed the catalog since it
    /// took it, and `replan` — the statement planned against the catalog committed then
    /// ([`WriterLock::foreign_grant_catalog`]) — does not fail the same way
    /// ([`pglet::plan_failure_is_the_writers`]). A failure of the statement's own (a
    /// syntax error, a column or a function no catalog has, a cast no catalog decides)
    /// answers at once, whatever another block is doing — where it is the first failure
    /// against both catalogs: a syntax error planning reaches only after a check the
    /// block's change decides (a set operation's column counts, an `INSERT`'s row length
    /// or its target list's columns) waits.
    pub fn plan_failure_is_the_writers<T>(
        &self,
        error: &pglet_sql::Error,
        generation: u64,
        replan: impl FnOnce(&Schema) -> pglet_sql::Result<T>,
    ) -> bool {
        self.shared
            .writer
            .foreign_grant_catalog(self.session.process_id, generation)
            .is_some_and(|committed| pglet::plan_failure_is_the_writers(error, &replan(&committed)))
    }

    /// Record what this connection found on taking the writer, under the guard it took
    /// `schema` with and before the statement changes anything: the catalog generation,
    /// and — where its block took the writer — the catalog committed then, which another
    /// connection's failing plan is planned against ([`WriterLock::note_grant`]).
    pub fn note_writer_grant(&self, schema: &Schema) {
        self.shared.writer.note_grant(
            self.session.process_id,
            schema.generation,
            self.tx.catalog_at_writer().cloned(),
        );
    }

    /// What a statement or a transaction command leaves behind once no block is open:
    /// the writer released (an auto-commit write held it for its statement, a block
    /// from its first write), and the session settings' block closed. Called wherever a
    /// statement or a block can end, and a no-op while a block is open, healthy or
    /// aborted — where it checks, in debug builds, that the block holds the writer
    /// exactly when its session says it took it.
    pub fn settle_writer(&mut self) {
        let pid = self.session.process_id;
        if self.transaction_status() == TransactionStatus::Idle {
            self.shared.writer.release(pid);
            // A block ends through `apply_transaction_command` or `rollback_transaction`,
            // each of which closes the settings' block first; one that ended any other way
            // is taken as rolled back.
            self.settings.end_block(false);
        } else {
            debug_assert_eq!(
                self.shared.writer.is_held_by(pid),
                self.tx.holds_writer(),
                "a block holds the writer exactly when its session took it"
            );
        }
    }

    /// Begin a transaction — the session snapshots what a rollback will restore
    /// (the catalog on every backend, the whole `MemoryStorage` on the backend that
    /// buffers no pending writes of its own) and tells storage to start.
    ///
    /// A `BEGIN` while a transaction is already open (`InTransaction` **or** the
    /// aborted `Failed` state) is a **no-op** — see [`TransactionSession::begin`] for
    /// why that early return is load-bearing rather than cosmetic. pglet doesn't
    /// surface Postgres's `WARNING: there is already a transaction in progress`
    /// (notices aren't wired), matching its *effect* — not its chatter.
    ///
    /// A storage failure is logged rather than propagated, as before: `BEGIN` has no
    /// error channel on this path, and the session is left `Idle` so nothing claims a
    /// block that never opened.
    pub fn begin_transaction(&mut self) {
        let conn_id = self.session.process_id;
        let (schema, mut storage) = self.shared.lock_both();
        if let Err(e) = self.tx.begin(conn_id, &schema, &mut storage) {
            eprintln!("warning: failed to begin storage transaction: {e}");
        }
    }

    /// Roll the transaction back — discard the buffered writes and restore the catalog
    /// ([`TransactionSession::rollback`]: whole for a block that took the writer,
    /// nothing for one that never wrote) — and release the writer. The transaction is
    /// always ended, even when a step fails, so the connection stays usable.
    ///
    /// # Errors
    ///
    /// The rollback's storage restore or its save.
    pub fn rollback_transaction(&mut self) -> Result<(), pglet_sql::Error> {
        let conn_id = self.session.process_id;
        let result = {
            let (mut schema, mut storage) = self.shared.lock_both();
            self.tx.rollback(conn_id, &mut schema, &mut storage)
        };
        self.settings.end_block(false);
        self.settle_writer();
        result
    }

    /// Apply one executed statement's transaction-lifecycle signal to this
    /// connection's block.
    ///
    /// The one entry point for all six commands, so the savepoint half cannot end up
    /// on a different path from `BEGIN`/`COMMIT`/`ROLLBACK`. `BEGIN` keeps its own
    /// method because it has no error channel on this path.
    ///
    /// # Errors
    ///
    /// Whatever the session raises: the commit's serialize/persist failure, either
    /// concurrent-DDL refusal, or a savepoint statement's `25P01` / `3B001`.
    pub fn apply_transaction_command(
        &mut self,
        command: &TransactionCommand,
    ) -> Result<(), pglet_sql::Error> {
        let was = self.tx.status();
        let result = if matches!(command, TransactionCommand::Begin(_)) {
            self.begin_transaction();
            Ok(())
        } else {
            let conn_id = self.session.process_id;
            let (mut schema, mut storage) = self.shared.lock_both();
            self.tx
                .apply_command(conn_id, command, &mut schema, &mut storage)
        };
        let carried = self
            .tx
            .carry_to_settings(&mut self.settings, command, was, result.is_ok());
        self.settle_writer();
        result.and(carried)
    }

    /// Mark the transaction as failed (only an exit statement is admitted after this),
    /// suspending the unique-key claims its staged writes hold (which no other
    /// connection reaches while the block holds the writer).
    ///
    /// **Locks nothing unless a healthy block is actually going aborted.** Every
    /// statement error routes here, the lock-free read path's plan errors included, and
    /// a connection with no open block has no claim to release — so the block's status
    /// is read first, and a failing `SELECT` on an idle connection goes on taking no
    /// lock at all. When it does lock, it takes storage alone: an abort reads no
    /// schema, and `lock_both` would serialize the failure behind whatever write holds
    /// the schema mutex.
    ///
    /// Deadlock-free by construction rather than by audit: the guard borrows
    /// `self.shared`, so a live one makes the `&mut self` every caller of this method
    /// holds a borrow error. A caller that still had the lock could not have called it.
    pub fn fail_transaction(&mut self) {
        if !matches!(self.tx.status(), TransactionStatus::InTransaction) {
            return;
        }
        let Self {
            shared,
            session,
            tx,
            ..
        } = self;
        let mut storage = shared.lock_storage();
        tx.fail(session.process_id, &mut storage);
    }

    /// Release any transaction still open when the connection is closing.
    ///
    /// Called on **every** exit path of the socket loop — graceful EOF, a client
    /// `Terminate`, and I/O / wire-decode error propagation — so a client that
    /// disconnects mid-transaction (crash, timeout, connection-pool recycle) can't
    /// leak its per-connection `BtreeTransactionState` and, with it, pin an MVCC
    /// snapshot version forever. A leaked snapshot keeps `min_active_version` from
    /// advancing, so vacuum/GC can never reclaim the retired pages and the file
    /// grows without bound — this is the documented "DISCONNECT (any state):
    /// rollback active transaction" contract. Both `InTransaction` and the aborted
    /// `Failed` state hold a frame; only `Idle` needs no cleanup.
    ///
    /// A cleanup rollback that itself fails is logged and swallowed: it must not
    /// mask the socket loop's own exit result, and `rollback_transaction` ends the
    /// transaction (returns to `Idle`, clears the frames) and releases the writer even
    /// when a step fails.
    pub fn cleanup_on_disconnect(&mut self) {
        if self.transaction_status() == TransactionStatus::Idle {
            self.settle_writer();
            return;
        }
        if let Err(e) = self.rollback_transaction() {
            eprintln!(
                "warning: rollback on disconnect failed (connection {}): {e}",
                self.session.process_id
            );
        }
    }

    /// Persist a just-applied auto-commit write so it survives a restart.
    ///
    /// A mutating statement that runs outside an explicit `BEGIN…COMMIT` flips
    /// in-memory tree state and writes CoW pages, but never advances the durable
    /// meta slot — so a restart loses it. This mirrors the explicit-`COMMIT`
    /// persist sequence (`commit_transaction`): write the durable meta slot with
    /// the **full** metadata via `save_state`, which assembles root, version,
    /// free_list, next_page_id, table_mappings, next_table_id, schema_bytes, and
    /// row_id_counters from live storage state (no partial hand-rolled copy that
    /// would drop the catalog).
    ///
    /// Like `commit_transaction` — whose `storage.commit(conn_id)` applies the
    /// pending-writes batch — an auto-commit write now goes through
    /// `apply_batch` (a one-element batch), which already fsyncs the data pages
    /// (fsync #1). So this only flips the meta slot via `save_state` (fsync #2,
    /// the durable commit point). Two fsyncs total, matching the embed `Database`
    /// auto-commit path and the bench's N=1 case. (No explicit `flush()` here —
    /// that would make a redundant third fsync.)
    ///
    /// Runs against the storage guard already held from the write's critical
    /// section, so no other writer can interleave between the write and its
    /// persist. The schema is serialized by the caller (which still holds the
    /// schema lock) and threaded through as `schema_bytes`, mirroring
    /// `commit_transaction`'s drop-schema-lock-before-storage-I/O ordering.
    ///
    /// Callers MUST gate this on "the statement mutated AND there is no active
    /// transaction for this connection" so read-only sessions never fsync and
    /// in-transaction writes stay deferred to the explicit `COMMIT`.
    /// Persist the schema/meta snapshot after an autocommit DDL/DML.
    ///
    /// # Errors
    ///
    /// Returns the storage error if the durable meta-flip (`save_state`) fails —
    /// the caller surfaces it to the client rather than reporting a success that
    /// wasn't made durable. This is the autocommit mirror of
    /// `commit_transaction`'s `save_state` propagation: a swallowed fsync failure
    /// here would silently lose a committed write while telling the client it
    /// succeeded.
    pub fn persist_autocommit(
        storage: &StorageBackend,
        schema_bytes: Vec<u8>,
    ) -> Result<(), pglet_sql::Error> {
        storage.save_state(schema_bytes).map_err(Into::into)
    }

    /// The auto-commit publish shared by the SQL write path (`execute::run_one`) and
    /// the COPY ingest (`copy::finish_copy_in`): encode the catalog, commit the staged
    /// writes, **drop the schema lock**, then flip the durable meta slot.
    ///
    /// Single-sourced so the load-bearing rules can't drift between the two callers:
    /// **nothing is published before the encoding** — a refusal there discards the staged
    /// writes ([`pglet::autocommit_publish_bytes`]), as a write the device refuses at a
    /// `discardable` commit does (below) — and **never persist on an encoding
    /// error** (a truncated/empty blob would flip the durable meta slot to an empty
    /// catalog, wiping every table on reopen).
    /// A commit or persist failure is returned loud so the caller surfaces it rather
    /// than reporting a success that wasn't made durable. The schema guard is consumed
    /// (dropped before the storage write); the still-held storage guard is borrowed.
    /// Each caller keeps its own "is this an auto-commit write" gate.
    ///
    /// # Errors
    ///
    /// Returns the encoding error (the statement unwound), the storage commit error,
    /// or the `save_state` durable-flip error. `discardable`
    /// ([`pglet::autocommit_is_discardable`]) lets a write the device refuses before
    /// anything is published drop the staged writes and leave the store serving
    /// (`Error::WriteRefused`).
    pub fn publish_autocommit(
        conn_id: i32,
        schema: MutexGuard<'_, Schema>,
        storage: &mut StorageBackend,
        discardable: bool,
    ) -> Result<(), pglet_sql::Error> {
        let schema_bytes = pglet::autocommit_publish_bytes(conn_id, &schema, storage)?;
        if discardable {
            storage.commit_discardable(conn_id)?;
        } else {
            storage.commit(conn_id)?;
        }
        drop(schema); // release the schema lock before the storage write
        Self::persist_autocommit(storage, schema_bytes)
    }
}

impl Drop for Connection {
    /// A connection that goes away — a thread that unwinds included — releases the
    /// writer if it holds it, so no statement it can no longer finish keeps every other
    /// writer waiting. The ordinary exits roll the block back first
    /// ([`Self::cleanup_on_disconnect`]), which releases it there.
    fn drop(&mut self) {
        self.shared.writer.release(self.session.process_id);
    }
}

/// Handle a client connection.
///
/// Runs the socket read/dispatch loop, then — on **every** way it can exit
/// (graceful EOF, a client `Terminate`, or an I/O / wire-decode error) — rolls
/// back any transaction the client left open via
/// [`Connection::cleanup_on_disconnect`], so a mid-transaction disconnect can't
/// leak an MVCC snapshot and wedge vacuum/GC. The loop's `Result` is captured,
/// not returned directly, so the cleanup runs on the error path too (a `?` inside
/// the loop would otherwise skip it) and never masks the loop's own result.
pub fn handle_connection(
    stream: TcpStream,
    pid: i32,
    shared: Arc<SharedState>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Seed this connection thread's unsupported-type policy from the server
    // config (the resolvers read a thread-local; the server is thread-per-
    // connection, so each thread sets it once here).
    pglet_sql::planner::set_coerce_unknown_types(shared.coerce_unknown_types());

    let mut conn = Connection::new(pid, shared);
    let result = run_connection_loop(&mut conn, &stream);
    conn.cleanup_on_disconnect();
    // Explicit: this function owns the socket and closing it is what ends the
    // connection for the client. The loop borrows it (`Read`/`Write` on
    // `&TcpStream`), so without this the ownership would read as accidental.
    drop(stream);
    result
}

/// The socket read/dispatch loop for one connection.
///
/// Returns when the client disconnects (EOF), sends `Terminate`, or an I/O /
/// wire-decode error occurs. Transaction cleanup on exit is the caller's job
/// ([`handle_connection`]), so *every* return here funnels through one rollback.
fn run_connection_loop(
    conn: &mut Connection,
    stream: &TcpStream,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut buf = vec![0u8; 16384]; // Start with 16KB for typical queries
    let mut buf_len = 0;
    // Responses go through a buffer, flushed before each read of the socket (and
    // written as it fills), so a query's `RowDescription` + N `DataRow` +
    // `CommandComplete` + `ReadyForQuery` reach the socket as one write rather than
    // four-plus when they fit the buffer.
    //
    // Bounded on purpose: a `BufWriter` and not a `Vec` the whole response is
    // built in, because a large `SELECT` streams — its rows must reach the
    // client as the buffer fills, not be held in memory until the last one is
    // encoded. The flush placement is the load-bearing part: every path that
    // stops writing and waits for the client to speak must flush first, or the
    // two ends deadlock, each waiting for the other. Each such path is marked
    // at its `flush()`.
    //
    // `Read`/`Write` are implemented for `&TcpStream`, so the writer borrows the
    // socket immutably and the read below borrows it the same way: one socket
    // serves both without a `try_clone`.
    let mut out = BufWriter::new(stream);

    loop {
        // Grow buffer if full (for large queries like bulk INSERTs)
        if buf_len == buf.len() {
            let new_size = buf.len().saturating_mul(2).min(64 * 1024 * 1024); // Cap at 64MB
            if new_size == buf.len() {
                return Err("message too large".into());
            }
            buf.resize(new_size, 0);
        }

        // Read more data. Flush first: this is where the loop stops writing and
        // waits for the client, so anything still buffered has to be on the wire
        // before the wait — the deadlock this whole buffer risks.
        out.flush()?;
        let n = (&*stream).read(&mut buf[buf_len..])?;
        if n == 0 {
            break; // EOF
        }
        buf_len += n;

        // Process messages
        loop {
            match decode_message(&buf[..buf_len], conn.session.is_startup()) {
                Ok((msg, consumed)) => {
                    // Shift buffer
                    buf.copy_within(consumed..buf_len, 0);
                    buf_len -= consumed;

                    // Shrink buffer if it grew large and we've processed the big message
                    if buf.len() > 32768 && buf_len < 8192 {
                        buf.truncate(16384);
                    }

                    // Handle SSL request specially (single byte response, stay in startup)
                    if matches!(msg, FrontendMessage::SslRequest) {
                        out.write_all(b"N")?; // SSL not supported
                        out.flush()?; // the client waits for this byte before its startup
                        continue;
                    }

                    // GSSAPI encryption request: decline exactly like SSL — reply a
                    // single `N` and stay in startup so the client can proceed with a
                    // plaintext startup message (or an SSLRequest). A libpq client with
                    // `gssencmode=prefer` sends this first; without declining it here
                    // the client reads a spurious `AuthenticationOk` and hangs.
                    if matches!(msg, FrontendMessage::GssEncRequest) {
                        out.write_all(b"N")?; // GSS encryption not supported
                        out.flush()?; // likewise — the client blocks on this byte
                        continue;
                    }

                    // Cancel request: pglet implements no query cancellation. This
                    // closes the packet's own connection with NO reply (the
                    // fire-and-forget contract) and reaches no session: the request
                    // names a backend (`pid`/`secret`), not a statement, and a
                    // statement running on that backend runs to completion and
                    // answers (`EDGES.md` row 224). Never enter startup or answer
                    // `AuthenticationOk`.
                    if matches!(msg, FrontendMessage::CancelRequest { .. }) {
                        return Ok(());
                    }

                    // Handle message
                    let responses = handle_message(conn, msg);
                    for resp in responses {
                        let encoded = encode_message(&resp)?;
                        out.write_all(&encoded)?;
                    }

                    if conn.session.state == SessionState::Terminated {
                        out.flush()?; // last write of the connection
                        return Ok(());
                    }
                }
                Err(pglet_wire::WireError::Incomplete(_)) => break, // need more data (the read above flushes)
                Err(e) => {
                    out.flush()?; // an error response may already be buffered
                    return Err(e.into());
                }
            }
        }
    }

    out.flush()?;
    Ok(())
}

#[cfg(test)]
mod persist_fault_tests {
    //! What a write or `fsync` the device refuses answers, at autocommit and at `COMMIT` —
    //! loud, never a reported success — and what the connections and a reopen of the image
    //! then read: a failed store ends every connection, and a refusal the statement can
    //! drop leaves the store serving. What a rollback saves — its restored catalog, its
    //! counters, never another connection's open DDL — the index and table numbers a
    //! reopen hands out, and a read in flight when the store fails.

    use std::sync::Arc;

    use pglet_testing::block_io::MemBlockIo;
    use pglet_testing::db::open_database;
    use pglet_wire::{BackendMessage, FrontendMessage, SessionState};

    use super::{Connection, SharedState};
    use crate::test_support::{has_error, run, sqlstate};

    fn has_command_complete(msgs: &[BackendMessage]) -> bool {
        msgs.iter()
            .any(|m| matches!(m, BackendMessage::CommandComplete { .. }))
    }

    fn conn_over(io: &MemBlockIo) -> Connection {
        let db = open_database(Box::new(io.clone()));
        let shared = Arc::new(SharedState::from_database(db));
        Connection::new(1, shared)
    }

    #[test]
    fn autocommit_persist_failure_is_loud() {
        let io = MemBlockIo::new();
        let mut conn = conn_over(&io);

        // A faithful autocommit DDL persists and reports success.
        let ok = run(&mut conn, "CREATE TABLE t (id INT)");
        assert!(
            has_command_complete(&ok) && !has_error(&ok),
            "faithful DDL should succeed: {ok:?}"
        );

        // Arm an I/O error on the durable meta write: the next autocommit DDL must
        // surface an ErrorResponse and NOT a CommandComplete.
        io.policy().arm_fail_slot_writes();
        let failed = run(&mut conn, "CREATE TABLE t2 (id INT)");
        assert!(
            has_error(&failed),
            "a failed autocommit persist must error: {failed:?}"
        );
        assert!(
            !has_command_complete(&failed),
            "a failed persist must not report CommandComplete: {failed:?}"
        );
    }

    fn query(conn: &mut Connection, sql: &str) -> Vec<BackendMessage> {
        crate::handlers::handle_message(conn, FrontendMessage::Query { sql: sql.into() })
    }

    /// After a durable write fails, the statement that met it answers `58030` and no
    /// connection is served from the failed state again: the next message on it, or on
    /// another one, is a `FATAL 57P02` that ends the connection, and a startup on it is a
    /// `FATAL 57P03` — what 18.6 answers after a `PANIC`, until recovery. The server's
    /// recovery, a new connection served from the reopened store, is `server.rs`'s.
    #[test]
    fn a_failed_durable_write_ends_every_connection() {
        let io = MemBlockIo::new();
        let mut conn = conn_over(&io);
        let mut other = Connection::new(2, Arc::clone(&conn.shared));
        run(&mut conn, "CREATE TABLE t (id INT)");
        io.policy().arm_fail_slot_writes();
        let failed = query(&mut conn, "INSERT INTO t VALUES (1)");
        io.policy().disarm();
        assert_eq!(sqlstate(&failed).as_deref(), Some("58030"), "{failed:?}");
        assert!(!conn.session.state.eq(&SessionState::Terminated));

        for (c, label) in [
            (&mut other, "another connection"),
            (&mut conn, "the same one"),
        ] {
            let refused = query(c, "SELECT * FROM t");
            assert_eq!(
                sqlstate(&refused).as_deref(),
                Some("57P02"),
                "{label}: {refused:?}"
            );
            assert!(is_fatal(&refused), "{label}: {refused:?}");
            assert_eq!(c.session.state, SessionState::Terminated, "{label}");
        }
        let mut fresh = Connection::new(3, Arc::clone(&conn.shared));
        let startup = crate::handlers::handle_message(
            &mut fresh,
            FrontendMessage::Startup {
                version: pglet_wire::protocol::PROTOCOL_VERSION_3_0,
                params: vec![("user".into(), "u".into())],
            },
        );
        assert_eq!(sqlstate(&startup).as_deref(), Some("57P03"), "{startup:?}");
        assert!(is_fatal(&startup), "{startup:?}");
        assert_eq!(fresh.session.state, SessionState::Terminated);
    }

    /// A block whose `COMMIT` meets a write refused before anything published ends rolled back,
    /// and the store serves on when that rollback takes the block's catalog change back — which
    /// it now always can: the next statement on another connection is served and the block's
    /// sequence is gone. Another connection's catalog DDL, committed after the block's `BEGIN`
    /// and before its first write, stays: the block took the writer re-based on it. (With DDL
    /// committed *during* a written block — which the writer now makes wait — the rollback used
    /// to refuse to restore, and the store failed.) A rollback that restores can fail the store
    /// where it must save what it restored and that save is refused; the embed leg's
    /// `a_refused_commit_past_a_save_of_its_ddl_is_not_answered_as_failed` carries that, since
    /// on the server only the block's writer saves while it is open.
    #[test]
    fn a_refused_commit_leaves_the_store_serving_when_its_rollback_restores_the_catalog() {
        for before_first_write in [false, true] {
            let label = if before_first_write {
                "another connection's DDL before the block's first write"
            } else {
                "alone"
            };
            let io = MemBlockIo::new();
            let mut conn = conn_over(&io);
            let mut other = Connection::new(2, Arc::clone(&conn.shared));
            run(&mut conn, "CREATE TABLE t (id INT)");
            run(&mut conn, "BEGIN");
            if before_first_write {
                let ddl = run(&mut other, "CREATE TABLE elsewhere (x INT)");
                assert!(!has_error(&ddl), "{label}: {ddl:?}");
            }
            run(&mut conn, "CREATE SEQUENCE q");
            run(&mut conn, "INSERT INTO t VALUES (1)");
            io.policy().arm_fill_disk_after(0);
            let committed = run(&mut conn, "COMMIT");
            io.policy().disarm();
            assert_eq!(
                sqlstate(&committed).as_deref(),
                Some("53100"),
                "{label}: {committed:?}"
            );
            let next = query(&mut other, "SELECT count(*) FROM t");
            assert!(!has_error(&next), "{label}: the store serves on: {next:?}");
            let q = query(&mut other, "SELECT nextval('q')");
            assert_eq!(sqlstate(&q).as_deref(), Some("42P01"), "{label}: {q:?}");
            if before_first_write {
                let kept = query(&mut other, "SELECT count(*) FROM elsewhere");
                assert!(!has_error(&kept), "{label}: the other DDL stays: {kept:?}");
            }
        }
    }

    /// The `M` field of the first `ErrorResponse`.
    fn error_message(msgs: &[BackendMessage]) -> Option<String> {
        msgs.iter().find_map(|m| match m {
            BackendMessage::ErrorResponse { fields } => fields
                .iter()
                .find(|(k, _)| *k == b'M')
                .map(|(_, v)| v.clone()),
            _ => None,
        })
    }

    /// Drive `sql` through the extended protocol as a driver sends one statement:
    /// `Parse`, `Bind`, `Execute`, `Sync`, each through the message handler.
    fn extended(conn: &mut Connection, sql: &str) -> Vec<BackendMessage> {
        let mut out = Vec::new();
        for msg in [
            FrontendMessage::Parse {
                name: String::new(),
                query: sql.into(),
                param_types: Vec::new(),
            },
            FrontendMessage::Bind {
                portal: String::new(),
                statement: String::new(),
                param_formats: Vec::new(),
                params: Vec::new(),
                result_formats: Vec::new(),
            },
            FrontendMessage::Execute {
                portal: String::new(),
                max_rows: 0,
            },
            FrontendMessage::Sync,
        ] {
            out.extend(crate::handlers::handle_message(conn, msg));
        }
        out
    }

    /// The server leg of `durability.rs`'s
    /// `a_refused_commit_past_a_save_of_its_ddl_is_not_answered_as_failed`, at the one save a
    /// server block can meet: another connection's write after the block's `BEGIN` and
    /// before its first write (after it the block holds the writer, and the server saves on
    /// nothing but the writer's statements — it has no `sync`). The block's `COMMIT` meets a
    /// full disk that persists; it is a healthy block's, or an aborted block's, which the
    /// server converts to a rollback at its aborted-block gate — one gate with two call
    /// sites, the simple protocol's statement loop and the extended protocol's pre-loop test,
    /// so the failing statement and the `COMMIT` go through each protocol.
    ///
    /// The discriminating assertion first: the answer says the outcome is unknown exactly
    /// when the reopen holds the sequence — here never. Then: nothing a save made durable is
    /// the block's, so the rollback has nothing to save — the healthy block's `COMMIT`
    /// answers the refused write (`WriteRefused`, `53100`) with the store serving on, and the
    /// aborted block's answers `ROLLBACK`. So it checks that the server routes the aborted
    /// block's `COMMIT` through the session's commit on both protocols.
    #[test]
    fn a_refused_commit_whose_ddl_no_save_reached_answers_the_definite_failure() {
        for (aborted, protocol) in [
            (false, "simple"),
            (false, "extended"),
            (true, "simple"),
            (true, "extended"),
        ] {
            let send = |conn: &mut Connection, sql: &str| {
                if protocol == "simple" {
                    run(conn, sql)
                } else {
                    extended(conn, sql)
                }
            };
            // Another connection's write before the block's first one is the one save a
            // server block can meet: after it the block holds the writer, and the server
            // saves on nothing but a writer's statement.
            {
                let save = "before the block";
                let label = format!(
                    "{} block, {protocol} protocol, the save {save}",
                    if aborted { "aborted" } else { "healthy" }
                );
                let io = MemBlockIo::new();
                let mut conn = conn_over(&io);
                let mut other = Connection::new(2, Arc::clone(&conn.shared));
                let mut other_saves = |label: &str| {
                    let saved = run(&mut other, "INSERT INTO t VALUES (7)");
                    assert!(!has_error(&saved), "{label}: {saved:?}");
                };
                run(&mut conn, "CREATE TABLE t (id INT)");
                run(&mut conn, "BEGIN");
                if save == "before the block" {
                    other_saves(&label);
                }
                run(&mut conn, "INSERT INTO t VALUES (1)");
                run(&mut conn, "CREATE SEQUENCE q");
                if aborted {
                    let bad = send(&mut conn, "SELECT 1 / 0");
                    assert!(has_error(&bad), "{label}: the block aborts: {bad:?}");
                }
                io.policy().arm_fill_disk_after(0);
                let committed = send(&mut conn, "COMMIT");
                io.policy().disarm();
                let message = error_message(&committed).unwrap_or_default();
                let unknown = message.starts_with("whether the statement persists is unknown");
                let mut reopened = open_database(Box::new(io.reopen()));
                let durable = reopened.execute("SELECT nextval('q')").is_ok();
                assert_eq!(
                    unknown, durable,
                    "{label}: unknown exactly when the reopen holds the sequence: {message}"
                );
                assert!(!durable, "{label}: no save reached the block's DDL");

                let next = query(&mut other, "SELECT count(*) FROM t");
                if durable {
                    assert_eq!(
                        sqlstate(&committed).as_deref(),
                        Some("53100"),
                        "{label}: {committed:?}"
                    );
                    assert_eq!(
                        sqlstate(&next).as_deref(),
                        Some("57P02"),
                        "{label}: {next:?}"
                    );
                } else {
                    if aborted {
                        assert!(!has_error(&committed), "{label}: {committed:?}");
                        assert!(
                            committed.iter().any(|m| matches!(
                                m,
                                BackendMessage::CommandComplete { tag } if tag == "ROLLBACK"
                            )),
                            "{label}: {committed:?}"
                        );
                    } else {
                        assert_eq!(
                            sqlstate(&committed).as_deref(),
                            Some("53100"),
                            "{label}: {committed:?}"
                        );
                        assert!(
                            message.starts_with("could not write to the database file"),
                            "{label}: {message}"
                        );
                    }
                    assert!(!has_error(&next), "{label}: the store serves on: {next:?}");
                }
            }
        }
    }

    /// The server leg of `durability.rs`'s
    /// `a_value_nextval_handed_out_is_not_handed_out_again_after_a_close`: a connection's
    /// block hands out a `nextval` and a `SERIAL` id, then ends in a rollback — its
    /// `ROLLBACK`, a `COMMIT` of the block aborted, or its client going away
    /// (`cleanup_on_disconnect`) — and a state reopened from the bytes the rollback leaves
    /// hands out the next values, not those again. The server has no clean shutdown of its
    /// own: a signal ends the process as a kill does, so what a reopen reads is what the
    /// last save wrote. Witnessed per cell: the bytes before the end reopen handing the
    /// values out again — today's mechanism, not the requirement (the repo's `EDGES.md` row
    /// 214), which a repair making an advance durable before its block ends turns red.
    #[test]
    fn a_value_nextval_handed_out_is_not_handed_out_again_after_a_reopen() {
        let next = |conn: &mut Connection| {
            [
                "SELECT nextval('s')",
                "INSERT INTO sr (x) VALUES (3) RETURNING id",
            ]
            .map(|sql| data_rows(&query(conn, sql)))
        };
        let reopen = |bytes: Vec<u8>| {
            let db = open_database(Box::new(MemBlockIo::from_bytes(bytes)));
            Connection::new(9, Arc::new(SharedState::from_database(db)))
        };
        for end in ["ROLLBACK", "COMMIT of an aborted block", "disconnect"] {
            let io = MemBlockIo::new();
            let mut a = conn_over(&io);
            for sql in [
                "CREATE SEQUENCE s",
                "SELECT nextval('s')",
                "CREATE TABLE sr (x INT CHECK (x > 0), id SERIAL)",
                "INSERT INTO sr (x) VALUES (1)",
                "BEGIN",
            ] {
                assert!(!has_error(&run(&mut a, sql)), "{end}: {sql}");
            }
            let handed = next(&mut a);
            let before = io.snapshot_bytes();
            assert_eq!(
                next(&mut reopen(before)),
                handed,
                "{end}: witness, the bytes before the end hand the values out again"
            );
            if end == "disconnect" {
                a.cleanup_on_disconnect();
            } else {
                if end != "ROLLBACK" {
                    assert!(has_error(&run(&mut a, "SELECT 1 / 0")), "{end}");
                }
                let sql = if end == "ROLLBACK" {
                    "ROLLBACK"
                } else {
                    "COMMIT"
                };
                assert!(!has_error(&run(&mut a, sql)), "{end}");
            }
            let after = next(&mut reopen(io.snapshot_bytes()));
            assert_ne!(after, handed, "{end}: the reopen hands out the next values");
            assert_eq!(
                after,
                [
                    vec![vec![Some("3".to_string())]],
                    vec![vec![Some("3".to_string())]]
                ],
                "{end}: the next values"
            );
        }
    }

    /// A block that drops a committed table and creates it again, then rolls back, leaves a
    /// restart the committed table and its rows — its own rollback's single-connection half.
    /// (When another connection could write into the new table during the block, the
    /// table-id counter stayed above the new id and a reopen gave the name to it; that write
    /// now waits for the block's writer.) Ends: `ROLLBACK`, and the client going away
    /// (`cleanup_on_disconnect`); another connection's autocommit write after the block is
    /// the save a restart reads.
    #[test]
    fn a_rolled_back_re_creation_leaves_a_restart_the_committed_table() {
        let one = |conn: &mut Connection, sql: &str| -> Result<Vec<Vec<Option<String>>>, String> {
            let msgs = query(conn, sql);
            sqlstate(&msgs).map_or_else(|| Ok(data_rows(&msgs)), Err)
        };
        let committed: Vec<Vec<Option<String>>> = ["1", "2", "3"]
            .iter()
            .map(|v| vec![Some((*v).to_string())])
            .collect();
        for mode in ["ROLLBACK", "disconnect"] {
            let io = MemBlockIo::new();
            let mut a = conn_over(&io);
            let mut b = Connection::new(2, Arc::clone(&a.shared));
            let step = |conn: &mut Connection, sql: &str| {
                assert!(!has_error(&run(conn, sql)), "{mode}: {sql}");
            };
            for sql in [
                "CREATE TABLE t (i INT)",
                "INSERT INTO t VALUES (1), (2), (3)",
                "CREATE TABLE g (k INT)",
                "BEGIN",
                "DROP TABLE t",
                "CREATE TABLE t (i INT)",
                "INSERT INTO t VALUES (42)",
            ] {
                step(&mut a, sql);
            }
            if mode == "disconnect" {
                a.cleanup_on_disconnect();
            } else {
                step(&mut a, "ROLLBACK");
            }
            assert_eq!(
                one(&mut b, "SELECT i FROM t ORDER BY i"),
                Ok(committed.clone()),
                "{mode}: live"
            );
            step(&mut b, "INSERT INTO g VALUES (1)");
            let db = open_database(Box::new(MemBlockIo::from_bytes(io.snapshot_bytes())));
            let mut r = Connection::new(9, Arc::new(SharedState::from_database(db)));
            assert_eq!(
                one(&mut r, "SELECT i FROM t ORDER BY i"),
                Ok(committed.clone()),
                "{mode}: after a restart"
            );
        }
    }

    /// The rows a query's `DataRow`s carry, as text.
    fn data_rows(msgs: &[BackendMessage]) -> Vec<Vec<Option<String>>> {
        msgs.iter()
            .filter_map(|m| match m {
                BackendMessage::DataRow { values } => Some(
                    values
                        .iter()
                        .map(|v| v.as_ref().map(|b| String::from_utf8_lossy(b).into_owned()))
                        .collect(),
                ),
                _ => None,
            })
            .collect()
    }

    /// The server's own read path under the embed guard's shape
    /// (`durability.rs`'s `a_read_in_flight_when_its_store_fails_never_answers_with_a_reopened_stores_page`):
    /// a query on a connection of the state whose store fails while one of its page reads
    /// is in flight, a state reopened over the same bytes rewriting that page before the
    /// read copies it (`FaultPolicy::arm_before_data_read`, on the reading thread). The
    /// query answers the durable rows or `57P02`, never the rewritten page's. Every cold
    /// read of an index lookup and of a scan is gated in turn; witnessed per cell, the page
    /// was rewritten. Measured with the second check removed: the scan's last gated read
    /// answered a count short of the table's.
    #[test]
    fn a_read_in_flight_when_its_store_fails_never_answers_with_a_reopened_stores_page() {
        use std::sync::atomic::{AtomicBool, Ordering};

        let io = MemBlockIo::new();
        {
            let mut conn = conn_over(&io);
            for sql in [
                "CREATE TABLE t (id INT PRIMARY KEY, v TEXT)",
                "CREATE INDEX t_v ON t (v)",
                "INSERT INTO t VALUES (1, 'a'), (5, 'e')",
                "CREATE TABLE big (id INT PRIMARY KEY, pad TEXT)",
            ] {
                assert!(!has_error(&run(&mut conn, sql)), "{sql}");
            }
            // Enough rows that the tree outgrows what an open reads, so the lookup's
            // pages are read cold.
            let rows: Vec<String> = (1..=200)
                .map(|k| format!("({k}, '{}')", "p".repeat(300)))
                .collect();
            let filled = run(
                &mut conn,
                &format!("INSERT INTO big VALUES {}", rows.join(", ")),
            );
            assert!(!has_error(&filled), "{filled:?}");
        }
        let base = io.snapshot_bytes();
        let mut cells = 0;
        for (sql, durable) in [
            (
                "SELECT id FROM t WHERE v = 'e'",
                vec![vec![Some("5".to_string())]],
            ),
            (
                "SELECT count(*), sum(id) FROM big",
                vec![vec![Some("200".to_string()), Some("20100".to_string())]],
            ),
        ] {
            for skip in 0.. {
                let io = MemBlockIo::from_bytes(base.clone());
                let db = open_database(Box::new(io.clone()));
                let failure = db.storage().write_failure().expect("a file-format store");
                let mut conn = Connection::new(1, Arc::new(SharedState::from_database(db)));
                let rewritten = Arc::new(AtomicBool::new(false));
                let hook_rewritten = Arc::clone(&rewritten);
                let shared = io.reopen();
                let policy = io.policy();
                policy.arm_before_data_read(
                    skip,
                    Box::new(move |offset| {
                        failure.record("a refused save on another connection".into());
                        let page = |bytes: &[u8]| {
                            let at = usize::try_from(offset).unwrap();
                            bytes.get(at..at + 4096).map(<[u8]>::to_vec)
                        };
                        let before = page(&shared.snapshot_bytes());
                        // What `Server::connection_state` serves the next connection from.
                        let reopened =
                            SharedState::from_database(open_database(Box::new(shared.reopen())));
                        let mut next = Connection::new(2, Arc::new(reopened));
                        run(&mut next, "DROP TABLE big");
                        run(&mut next, "CREATE TABLE x (id INT PRIMARY KEY, pad TEXT)");
                        for i in 0..100 {
                            let rows: Vec<String> = (0..20)
                                .map(|j| format!("({}, '{}')", i * 20 + j, "q".repeat(300)))
                                .collect();
                            let wrote = run(
                                &mut next,
                                &format!("INSERT INTO x VALUES {}", rows.join(", ")),
                            );
                            assert!(!has_error(&wrote), "{wrote:?}");
                            if page(&shared.snapshot_bytes()) != before {
                                hook_rewritten.store(true, Ordering::SeqCst);
                                return;
                            }
                        }
                    }),
                );
                let got = query(&mut conn, sql);
                if policy.before_data_read_armed() {
                    assert!(skip > 0, "{sql}: the query reads a page cold");
                    break;
                }
                cells += 1;
                assert!(
                    rewritten.load(Ordering::SeqCst),
                    "{sql}, cold read {skip}: the reopened state rewrote the page"
                );
                assert!(
                    sqlstate(&got).as_deref() == Some("57P02")
                        || (!has_error(&got) && data_rows(&got) == durable),
                    "{sql}, cold read {skip}: answered {got:?}"
                );
            }
        }
        assert!(cells >= 2, "each query was gated at least once: {cells}");
    }

    fn is_fatal(msgs: &[BackendMessage]) -> bool {
        msgs.iter().any(|m| {
            matches!(m, BackendMessage::ErrorResponse { fields }
                if fields.iter().any(|(k, v)| *k == b'S' && v == "FATAL"))
        })
    }

    #[test]
    fn explicit_commit_persist_failure_is_loud() {
        let io = MemBlockIo::new();
        let mut conn = conn_over(&io);

        run(&mut conn, "CREATE TABLE t (id INT)");
        run(&mut conn, "BEGIN");
        run(&mut conn, "INSERT INTO t VALUES (1)");

        // The durable persist is at COMMIT; with the meta write armed to fail, the
        // COMMIT must error (the client must ROLLBACK), not report success.
        io.policy().arm_fail_slot_writes();
        let committed = run(&mut conn, "COMMIT");
        assert!(
            has_error(&committed),
            "a COMMIT that can't persist must error: {committed:?}"
        );
        assert!(
            !has_command_complete(&committed),
            "a failed COMMIT must not report success: {committed:?}"
        );
    }
}

#[cfg(test)]
mod disconnect_cleanup_tests {
    //! A client that drops its connection mid-transaction (crash, timeout,
    //! connection-pool recycle) must not leak its per-connection transaction state
    //! or the MVCC snapshot version it pins — a leaked snapshot keeps
    //! `min_active_version` from advancing, so vacuum/GC can never reclaim the
    //! retired pages and the file grows without bound. `handle_connection` funnels
    //! every socket-loop exit through `Connection::cleanup_on_disconnect`; these
    //! tests drive that cleanup directly (no socket) and prove the transaction is
    //! rolled back — its buffered work is invisible to a fresh connection, and its
    //! `BtreeTransactionState` (with the pinned snapshot) is gone from shared
    //! storage. Covers **both** snapshot-holding states: `InTransaction` and the
    //! aborted `Failed`.

    use std::sync::Arc;

    use pglet_testing::block_io::MemBlockIo;
    use pglet_testing::db::open_database;
    use pglet_wire::TransactionStatus;

    use super::{Connection, SharedState};
    use crate::test_support::{has_error, run};

    /// A shared in-memory double-meta btree state (real MVCC snapshots + per-conn
    /// transaction tracking, the path that leaks on an uncleaned disconnect).
    fn shared_btree() -> Arc<SharedState> {
        let db = open_database(Box::new(MemBlockIo::new()));
        Arc::new(SharedState::from_database(db))
    }

    #[test]
    fn disconnect_in_transaction_rolls_back() {
        let shared = shared_btree();
        let mut conn1 = Connection::new(1, Arc::clone(&shared));

        run(&mut conn1, "BEGIN");
        run(&mut conn1, "CREATE TABLE t (id INT)");
        run(&mut conn1, "INSERT INTO t VALUES (1)");

        // Pre-condition: the transaction is open and pins a snapshot in storage.
        assert_eq!(conn1.transaction_status(), TransactionStatus::InTransaction);
        assert_eq!(conn1.tx.frame_depth(), 1);
        assert!(
            shared.storage.lock().has_transaction(1),
            "storage should track conn1's open transaction before cleanup"
        );

        // Simulate the mid-transaction disconnect (the socket-loop exit path).
        conn1.cleanup_on_disconnect();

        // Connection state reset and the snapshot released.
        assert_eq!(conn1.transaction_status(), TransactionStatus::Idle);
        assert_eq!(conn1.tx.frame_depth(), 0);
        assert!(
            !shared.storage.lock().has_transaction(1),
            "cleanup must release conn1's BtreeTransactionState (and its pinned snapshot)"
        );

        // A fresh connection must not see the rolled-back work: the immediate
        // CREATE TABLE was undone, so `t` no longer exists.
        let mut conn2 = Connection::new(2, Arc::clone(&shared));
        let sel = run(&mut conn2, "SELECT * FROM t");
        assert!(
            has_error(&sel),
            "rolled-back CREATE TABLE must leave `t` invisible to a fresh connection: {sel:?}"
        );
    }

    #[test]
    fn disconnect_in_failed_transaction_rolls_back() {
        let shared = shared_btree();
        let mut conn1 = Connection::new(1, Arc::clone(&shared));

        run(&mut conn1, "CREATE TABLE t (id INT)"); // autocommit; survives the rollback
        run(&mut conn1, "BEGIN");
        run(&mut conn1, "INSERT INTO t VALUES (1)");
        // An error inside the transaction moves it to the aborted `Failed` state,
        // which still holds the snapshot and so must also be cleaned up.
        let err = run(&mut conn1, "SELECT * FROM does_not_exist");
        assert!(has_error(&err), "bad statement should error: {err:?}");
        assert_eq!(conn1.transaction_status(), TransactionStatus::Failed);
        assert!(
            shared.storage.lock().has_transaction(1),
            "a Failed transaction still pins its snapshot in storage"
        );

        conn1.cleanup_on_disconnect();

        assert_eq!(conn1.transaction_status(), TransactionStatus::Idle);
        assert_eq!(conn1.tx.frame_depth(), 0);
        assert!(
            !shared.storage.lock().has_transaction(1),
            "cleanup must release a Failed transaction's snapshot too"
        );

        // The buffered INSERT was discarded; the autocommit table still exists and
        // is queryable (its one row rolled back).
        let mut conn2 = Connection::new(2, Arc::clone(&shared));
        let cnt = run(&mut conn2, "SELECT COUNT(*) FROM t");
        assert!(
            !has_error(&cnt),
            "autocommit table `t` should survive the rollback: {cnt:?}"
        );
    }
}

#[cfg(test)]
mod transaction_state_tests {
    //! Two transaction-state-machine bugs, driven through `execute_query_core` over a
    //! real in-memory double-meta btree (the same MVCC + per-connection transaction
    //! tracking the server uses), no socket:
    //!
    //! - A nested/redundant `BEGIN` while a transaction is already open must be a
    //!   no-op, preserving the DDL-rollback baseline so a later `ROLLBACK` unwinds
    //!   from the *first* BEGIN and the connection-level catalog stays in sync with
    //!   storage (Postgres: `WARNING: there is already a transaction in progress`,
    //!   otherwise unchanged).
    //! - `COMMIT`/`END` in an aborted transaction performs an implicit `ROLLBACK`
    //!   (command tag `ROLLBACK`, returns to Idle, pre-error writes discarded) rather
    //!   than re-raising `25P02` and wedging the transaction open forever.

    use std::sync::Arc;

    use pglet_testing::block_io::MemBlockIo;
    use pglet_testing::db::open_database;
    use pglet_wire::{BackendMessage, TransactionStatus};

    use super::{Connection, SharedState};
    use crate::test_support::{has_error, run, sqlstate};

    fn shared_btree() -> Arc<SharedState> {
        let db = open_database(Box::new(MemBlockIo::new()));
        Arc::new(SharedState::from_database(db))
    }

    fn command_tag(msgs: &[BackendMessage]) -> Option<String> {
        msgs.iter().find_map(|m| match m {
            BackendMessage::CommandComplete { tag } => Some(tag.clone()),
            _ => None,
        })
    }

    fn rfq_status(msgs: &[BackendMessage]) -> Option<TransactionStatus> {
        msgs.iter().find_map(|m| match m {
            BackendMessage::ReadyForQuery { status } => Some(*status),
            _ => None,
        })
    }

    /// The count returned by a `SELECT count(*)` (first row, first column, text).
    fn select_count(conn: &mut Connection, sql: &str) -> i64 {
        let msgs = run(conn, sql);
        assert!(!has_error(&msgs), "count query errored: {msgs:?}");
        let row = msgs
            .iter()
            .find_map(|m| match m {
                BackendMessage::DataRow { values } => Some(values.clone()),
                _ => None,
            })
            .expect("a DataRow");
        let bytes = row.first().and_then(|v| v.as_ref()).expect("a value");
        String::from_utf8_lossy(bytes)
            .parse()
            .expect("an integer count")
    }

    // ---- Bug 1: nested BEGIN preserves the DDL-rollback baseline ----

    #[test]
    fn nested_begin_preserves_rollback_baseline_catalog_and_storage_stay_synced() {
        // The exact desync repro: a nested BEGIN inside a DDL transaction must NOT
        // reset the session's frame (its catalog snapshot), so ROLLBACK undoes from the
        // first BEGIN and the connection-level catalog matches storage afterward.
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "BEGIN");
        run(&mut conn, "CREATE TABLE d_x(id int)");
        run(&mut conn, "INSERT INTO d_x VALUES (1)");

        // Redundant BEGIN: Postgres warns + no-ops. pglet reports the BEGIN tag and
        // must leave the transaction (and its baseline) unchanged.
        let nested = run(&mut conn, "BEGIN");
        assert_eq!(command_tag(&nested).as_deref(), Some("BEGIN"));
        assert!(
            !has_error(&nested),
            "nested BEGIN must not error: {nested:?}"
        );
        assert_eq!(conn.transaction_status(), TransactionStatus::InTransaction);

        let rolled = run(&mut conn, "ROLLBACK");
        assert_eq!(command_tag(&rolled).as_deref(), Some("ROLLBACK"));
        assert_eq!(conn.transaction_status(), TransactionStatus::Idle);

        // Catalog agrees `d_x` is gone: the re-CREATE (with a different shape)
        // succeeds instead of erroring `42P07 relation already exists`.
        let recreate = run(&mut conn, "CREATE TABLE d_x(id int, extra text)");
        assert!(
            !has_error(&recreate),
            "catalog desynced — re-CREATE after rollback errored: {recreate:?}"
        );

        // Storage agrees too: the table is queryable and empty (the rolled-back row
        // is gone), so catalog and storage are in sync.
        assert_eq!(select_count(&mut conn, "SELECT count(*) FROM d_x"), 0);
    }

    #[test]
    fn nested_begin_dml_only_commit_unwinds_from_first_begin() {
        // No DDL in the transaction: a nested BEGIN is still a no-op and COMMIT
        // publishes both inserts from the first BEGIN.
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "CREATE TABLE t(id int)"); // autocommit
        run(&mut conn, "BEGIN");
        run(&mut conn, "INSERT INTO t VALUES (1)");
        run(&mut conn, "BEGIN"); // no-op
        run(&mut conn, "INSERT INTO t VALUES (2)");
        let committed = run(&mut conn, "COMMIT");
        assert_eq!(command_tag(&committed).as_deref(), Some("COMMIT"));
        assert_eq!(conn.transaction_status(), TransactionStatus::Idle);
        assert_eq!(select_count(&mut conn, "SELECT count(*) FROM t"), 2);
    }

    #[test]
    fn nested_begin_dml_only_rollback_unwinds_from_first_begin() {
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "CREATE TABLE t(id int)"); // autocommit
        run(&mut conn, "BEGIN");
        run(&mut conn, "INSERT INTO t VALUES (1)");
        run(&mut conn, "BEGIN"); // no-op
        run(&mut conn, "INSERT INTO t VALUES (2)");
        let rolled = run(&mut conn, "ROLLBACK");
        assert_eq!(command_tag(&rolled).as_deref(), Some("ROLLBACK"));
        assert_eq!(conn.transaction_status(), TransactionStatus::Idle);
        // Both inserts rolled back (the table survives — it was autocommitted).
        assert_eq!(select_count(&mut conn, "SELECT count(*) FROM t"), 0);
    }

    #[test]
    fn begin_in_aborted_transaction_does_not_clear_the_failed_state() {
        // A BEGIN while aborted is rejected (25P02) and must leave the transaction
        // aborted — not silently reopened. (Bug 1's `begin_transaction` Failed-state
        // guard is the belt-and-suspenders that a caller reaching it wouldn't clear
        // the baseline either.)
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "CREATE TABLE t(id int)");
        run(&mut conn, "BEGIN");
        run(&mut conn, "INSERT INTO t VALUES (1)");
        let boom = run(&mut conn, "SELECT * FROM does_not_exist");
        assert!(has_error(&boom));
        assert_eq!(conn.transaction_status(), TransactionStatus::Failed);

        let begin = run(&mut conn, "BEGIN");
        assert_eq!(
            sqlstate(&begin).as_deref(),
            Some("25P02"),
            "BEGIN while aborted must stay 25P02: {begin:?}"
        );
        assert_eq!(
            conn.transaction_status(),
            TransactionStatus::Failed,
            "BEGIN must not clear the aborted state"
        );
    }

    // ---- Bug 2: COMMIT/END in an aborted transaction is an implicit ROLLBACK ----

    #[test]
    fn commit_in_aborted_transaction_rolls_back_reports_rollback_and_returns_idle() {
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "CREATE TABLE t(id int)"); // autocommit; survives
        run(&mut conn, "BEGIN");
        run(&mut conn, "INSERT INTO t VALUES (1)"); // pre-error write, must be discarded
        let boom = run(&mut conn, "SELECT 1/0");
        assert!(has_error(&boom), "1/0 should error: {boom:?}");
        assert_eq!(conn.transaction_status(), TransactionStatus::Failed);

        let committed = run(&mut conn, "COMMIT");
        assert!(
            !has_error(&committed),
            "COMMIT in an aborted txn must not error: {committed:?}"
        );
        assert_eq!(
            command_tag(&committed).as_deref(),
            Some("ROLLBACK"),
            "the implicit rollback reports the ROLLBACK tag, not COMMIT"
        );
        assert_eq!(rfq_status(&committed), Some(TransactionStatus::Idle));
        assert_eq!(conn.transaction_status(), TransactionStatus::Idle);

        // The pre-error INSERT was discarded (table survives — autocommitted, empty).
        assert_eq!(select_count(&mut conn, "SELECT count(*) FROM t"), 0);
    }

    #[test]
    fn end_alias_in_aborted_transaction_is_an_implicit_rollback() {
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "BEGIN");
        let boom = run(&mut conn, "SELECT 1/0");
        assert!(has_error(&boom));
        assert_eq!(conn.transaction_status(), TransactionStatus::Failed);

        let ended = run(&mut conn, "END");
        assert!(
            !has_error(&ended),
            "END in an aborted txn must recover: {ended:?}"
        );
        assert_eq!(command_tag(&ended).as_deref(), Some("ROLLBACK"));
        assert_eq!(rfq_status(&ended), Some(TransactionStatus::Idle));
        assert_eq!(conn.transaction_status(), TransactionStatus::Idle);
    }

    #[test]
    fn non_end_command_in_aborted_transaction_still_25p02() {
        // The exemption is narrow: a non-COMMIT/END/ROLLBACK statement still errors.
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "BEGIN");
        run(&mut conn, "SELECT 1/0");
        assert_eq!(conn.transaction_status(), TransactionStatus::Failed);

        let sel = run(&mut conn, "SELECT 1");
        assert_eq!(sqlstate(&sel).as_deref(), Some("25P02"));
        assert_eq!(rfq_status(&sel), Some(TransactionStatus::Failed));
        assert_eq!(conn.transaction_status(), TransactionStatus::Failed);
    }

    // ---- Happy paths must still work ----

    #[test]
    fn happy_path_commit_still_commits() {
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "CREATE TABLE t(id int)");
        run(&mut conn, "BEGIN");
        run(&mut conn, "INSERT INTO t VALUES (1)");
        let committed = run(&mut conn, "COMMIT");
        assert_eq!(command_tag(&committed).as_deref(), Some("COMMIT"));
        assert_eq!(rfq_status(&committed), Some(TransactionStatus::Idle));
        assert_eq!(select_count(&mut conn, "SELECT count(*) FROM t"), 1);
    }

    #[test]
    fn literal_rollback_in_aborted_transaction_still_works() {
        let shared = shared_btree();
        let mut conn = Connection::new(1, shared);

        run(&mut conn, "CREATE TABLE t(id int)");
        run(&mut conn, "BEGIN");
        run(&mut conn, "INSERT INTO t VALUES (1)");
        run(&mut conn, "SELECT 1/0");
        assert_eq!(conn.transaction_status(), TransactionStatus::Failed);

        let rolled = run(&mut conn, "ROLLBACK");
        assert_eq!(command_tag(&rolled).as_deref(), Some("ROLLBACK"));
        assert_eq!(rfq_status(&rolled), Some(TransactionStatus::Idle));
        assert_eq!(conn.transaction_status(), TransactionStatus::Idle);
        assert_eq!(select_count(&mut conn, "SELECT count(*) FROM t"), 0);
    }
}

#[cfg(test)]
mod negotiation_packet_tests {
    //! The pre-startup special packets (`GSSENCRequest`, `CancelRequest`) drive
    //! [`handle_connection`] over a real loopback socket and assert the exact bytes
    //! the server writes back — the SQL fixtures can't reach this raw-protocol path.
    //!
    //! - A `GSSENCRequest` (a common libpq default with `gssencmode=prefer`, sent
    //!   *first*) must be declined with a single `N` (like `SSLRequest`) and leave the
    //!   connection in startup, so a following `Startup` still yields
    //!   `AuthenticationOk`; read as a startup message instead, it would draw a spurious
    //!   `AuthenticationOk` and desync the client.
    //! - A `CancelRequest` (psql Ctrl-C / a driver query-timeout) is fire-and-forget:
    //!   the server must send **nothing** and close the connection, never reading it as
    //!   a startup message and answering `AuthenticationOk`. The request names a backend,
    //!   not a statement; pglet cancels nothing, so a statement running on that backend
    //!   runs on.

    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpListener, TcpStream};
    use std::sync::Arc;
    use std::thread;
    use std::time::Duration;

    use pglet_wire::protocol::{CANCEL_REQUEST_CODE, GSSAPI_ENC_REQUEST_CODE};

    use super::{SharedState, handle_connection};

    /// Bind an ephemeral loopback port and serve exactly one connection through
    /// [`handle_connection`] over an in-memory CoW-btree engine. Returns the address
    /// to connect a raw client to.
    fn spawn_one_shot_server() -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let addr = listener.local_addr().expect("local_addr");
        let shared = Arc::new(SharedState::with_btree());
        thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                let _ = handle_connection(stream, 1, shared);
            }
        });
        addr
    }

    /// The 8-byte GSSENCRequest packet: length `8` + the GSSAPI request code.
    fn gss_request() -> Vec<u8> {
        let mut buf = 8i32.to_be_bytes().to_vec();
        buf.extend_from_slice(&GSSAPI_ENC_REQUEST_CODE.to_be_bytes());
        buf
    }

    /// The 16-byte CancelRequest packet: length `16` + the cancel code + pid + secret.
    fn cancel_request(pid: i32, secret: i32) -> Vec<u8> {
        let mut buf = 16i32.to_be_bytes().to_vec();
        buf.extend_from_slice(&CANCEL_REQUEST_CODE.to_be_bytes());
        buf.extend_from_slice(&pid.to_be_bytes());
        buf.extend_from_slice(&secret.to_be_bytes());
        buf
    }

    /// A minimal protocol-3.0 Startup message (`user=postgres`), enough for the
    /// server's Startup arm to answer `AuthenticationOk`.
    fn startup_message() -> Vec<u8> {
        let params: &[u8] = b"user\0postgres\0\0";
        let total = i32::try_from(4 + 4 + params.len()).unwrap();
        let mut buf = total.to_be_bytes().to_vec();
        buf.extend_from_slice(&196_608i32.to_be_bytes()); // protocol 3.0
        buf.extend_from_slice(params);
        buf
    }

    #[test]
    fn gss_request_declined_with_n_then_startup_proceeds() {
        let addr = spawn_one_shot_server();
        let mut client = TcpStream::connect(addr).expect("connect");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");

        client.write_all(&gss_request()).expect("write GSS request");

        // Exactly one byte back: 'N' (GSS declined), byte-identical to the SSL decline
        // and to a Postgres backend built without GSSAPI.
        let mut one = [0u8; 1];
        client.read_exact(&mut one).expect("read GSS decline byte");
        assert_eq!(
            one[0], b'N',
            "GSSENCRequest must be declined with a single 'N'"
        );

        // The connection stayed in startup: a following Startup yields the 'R'
        // (AuthenticationOk) tag byte, proving no protocol desync.
        client.write_all(&startup_message()).expect("write Startup");
        let mut tag = [0u8; 1];
        client.read_exact(&mut tag).expect("read post-startup tag");
        assert_eq!(
            tag[0], b'R',
            "Startup after a GSS decline must yield AuthenticationOk ('R')"
        );
    }

    #[test]
    fn cancel_request_gets_no_reply_and_closes() {
        let addr = spawn_one_shot_server();
        let mut client = TcpStream::connect(addr).expect("connect");
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("set read timeout");

        client
            .write_all(&cancel_request(4242, 99))
            .expect("write CancelRequest");

        // The server must send NOTHING and close the connection — the read returns EOF
        // (0 bytes), never a spurious AuthenticationOk. handle_connection returns after
        // a CancelRequest, dropping its stream and sending TCP FIN.
        let mut buf = [0u8; 16];
        let n = client.read(&mut buf).expect("read after CancelRequest");
        assert_eq!(
            n,
            0,
            "CancelRequest must get no reply and a closed connection, got {n} bytes: {:?}",
            &buf[..n]
        );
    }
}
