//! The transaction block a session holds: its status, its frame stack, and the
//! classifier that decides which statements an aborted block still admits.
//!
//! # Why a value type rather than a `Database` method
//!
//! Two hosts drive the same state machine and neither owns the other's storage
//! handle. The embed facade ([`crate::Database`]) holds `schema` and `storage` as
//! fields; the native server never holds a `Database` at all — it calls
//! `into_parts()` at startup and puts the two halves behind separate mutexes, one
//! session per socket. So the unit that moves between them is a value with methods
//! over `(&mut Schema, &mut StorageBackend, conn_id)`, taking the two halves as
//! arguments rather than owning them. Written as a `Database` method it would be
//! unreachable from the server; written twice it would be the duplicated-rules seam
//! this workspace keeps paying for — and the two copies had already drifted into a
//! divergence about **committed data**, the wasm leg committing rows the native
//! server discarded.
//!
//! # Why a frame *stack*
//!
//! A savepoint is another frame: `SAVEPOINT` pushes, `RELEASE` pops, `ROLLBACK TO`
//! unwinds to one. The stack is what makes `frames[0]` — the block's own frame,
//! the only nameless one — unreachable by either of those lookups, which is what
//! keeps a `ROLLBACK TO` from unwinding the whole block.
//!
//! # What a frame holds, and why each field is separate
//!
//! Rolling a frame back has to undo two different things, which share no mechanism:
//!
//! - **storage** — the B+tree buffers pending writes in its own per-connection
//!   state, so `StorageBackend::rollback` discards them; `MemoryStorage` has no
//!   such buffer, so the frame carries a whole clone of it and the rollback
//!   restores that.
//! - **the catalog** — no backend rolls back the `Schema`, so the frame carries a
//!   clone of it and the rollback restores the *content*, advancing the `generation`
//!   stamp past the live one where anything moved it since the frame opened, or where
//!   the host borrowed the catalog during the block (see [`TransactionSession::rollback`]).
//!   **That clone cannot fail**, and a catalog deeper than the calling thread's stack
//!   walks overflowed it rather than refusing — so every host refuses a statement
//!   `54001` up front where this thread could not walk the catalog
//!   (`pglet_sql::depth::CatalogDepth`, `Database::enter`), and what is left
//!   here is a clone the check has already vouched for.
//!
//! # The writer, and why every rollback restores unconditionally
//!
//! At most one connection's block stages writes at a time: a block takes **the
//! writer** at its first write ([`TransactionSession::take_writer`]) and holds it until
//! it ends, and an auto-commit write holds it for its statement. The hosts enforce the
//! "at most one" — the native server makes a second writer wait, the embed facade
//! refuses it — and this session records whether its block has taken it. When it
//! does, every frame is **re-based** on the state committed at that moment, which is
//! sound because nothing is staged yet. From then on nothing but this block changes
//! the catalog or the store, so a rollback at any level restores its frame whole and
//! exactly; a block that never took the writer changed nothing and restores nothing,
//! since its frames are older than what other connections committed meanwhile.

use std::sync::Arc;

use pglet_core::{Error, TransactionStatus};
use pglet_sql::catalog::{Schema, Sequence, SessionSettings, deserialize_schema, serialize_schema};
use pglet_sql::executor::{
    QueryResult, TransactionCommand, enums_moved_by_restore, rebuild_indexes_over_enums,
};
use pglet_sql::lexer::{Keyword, TokenKind, is_word, lead_tokens};
#[cfg(feature = "memory_storage")]
use pglet_storage::MemoryStorage;

use crate::backend::{StorageAdapter, StorageBackend};

/// What a frame holds of the store itself: a whole `MemoryStorage`, the one backend
/// with no pending-write buffer of its own. Without the `memory_storage` feature there
/// is no such backend, and the type is uninhabited so the frame carries nothing.
#[cfg(feature = "memory_storage")]
type StoreSnapshot = MemoryStorage;
#[cfg(not(feature = "memory_storage"))]
type StoreSnapshot = std::convert::Infallible;

/// The whole-store snapshot `BEGIN` takes, on the backend that needs one — its data,
/// never another connection's savepoint levels.
fn store_snapshot(storage: &StorageBackend) -> Option<StoreSnapshot> {
    match storage {
        #[cfg(feature = "memory_storage")]
        StorageBackend::Memory(mem) => Some(mem.data_snapshot()),
        StorageBackend::Btree(_) => None,
    }
}

/// Put the block's frame snapshot back over the store — on memory storage, its data
/// alone, so every connection's savepoint levels stand; the B+tree's own `rollback`
/// already discarded the block's writes, and it carries none.
fn restore_store_snapshot(storage: &mut StorageBackend, snapshot: Option<StoreSnapshot>) {
    match storage {
        #[cfg(feature = "memory_storage")]
        StorageBackend::Memory(mem) => {
            if let Some(snapshot) = snapshot {
                mem.restore_data(snapshot);
            }
        }
        StorageBackend::Btree(_) => {
            let _ = snapshot;
        }
    }
}

/// Put the storage index specs back in step with `schema` after a restore moved the
/// enums `moved` names — the backward half of the rule
/// `executor::rebuild_indexes_over_enums` states.
///
/// The pair it completes is `executor::enums_moved_by_restore`, read **before** the
/// restore; that function's own docs enumerate every backward caller, this crate's two
/// among them.
///
/// The rebuilds go through `conn_id`'s [`StorageAdapter`], as a statement's do, so a
/// `ROLLBACK TO` — after which the block goes on — reads the entries they publish; after
/// a whole-block rollback no transaction is open and the adapter adds nothing.
fn resync_index_specs(
    schema: &Schema,
    storage: &mut StorageBackend,
    conn_id: i32,
    moved: &[String],
) -> Result<(), Error> {
    if moved.is_empty() {
        return Ok(());
    }
    rebuild_indexes_over_enums(schema, &mut StorageAdapter::new(storage, conn_id), moved)
}

/// The catalog's encoding, for the storage to persist beside a commit's data — empty
/// where nothing reads what that write leaves ([`StorageBackend::persists_catalog`]):
/// memory storage, whose `save_state` ignores it, and an in-memory B+tree private to one
/// connection, whose catalog write it makes a no-op.
///
/// Every commit asks for it **before** it publishes. The encoding takes the calling
/// thread's depth bounds a level (`pglet_sql::depth::EncodingLevel`), and a host's
/// entry check (`pglet_sql::depth::check_catalog_walks`) is what makes sure, before the
/// statement runs, that those bounds hold the catalog — so a commit's encoding is not
/// where a catalog too deep for the thread is refused.
///
/// # Errors
///
/// The encoding's error.
pub fn catalog_bytes(schema: &Schema, storage: &StorageBackend) -> Result<Vec<u8>, Error> {
    if storage.persists_catalog() {
        #[cfg(test)]
        crate::tests::count_catalog_encode();
        serialize_schema(schema)
    } else {
        Ok(Vec::new())
    }
}

/// Whether an auto-commit statement's staged writes are the whole of it.
///
/// When they are, a write the device refuses at their commit can drop them and leave the
/// database serving (`StorageBackend::commit_discardable`): the statement is of no
/// catalog-writing kind (`Plan::mutates_catalog`), and a catalog change is what an
/// auto-commit statement has no way to take back. A sequence advance — `nextval`, a
/// `SERIAL` default — is not one: it stands, as it does after a rollback on 18.6 too.
/// Asked of the plan and the catalog **before** the statement runs, as
/// `Plan::mutates_catalog` requires.
///
/// Every host's auto-commit SQL write asks it; a `COPY … FROM` writes the catalog only as
/// the sequence advances its column defaults make, and asks for the drop outright.
#[must_use]
pub fn autocommit_is_discardable(plan: &pglet_sql::planner::Plan, schema: &Schema) -> bool {
    !plan.mutates_catalog(schema)
}

/// The catalog's encoding for an auto-commit statement's publish.
///
/// Taken after the statement ran and **before** the storage commit publishes its staged
/// writes; where the encoding fails, the staged writes are discarded and the error
/// returned.
///
/// Every host's auto-commit write calls it. Nothing puts the catalog back: a
/// statement's catalog change stands in the session if the encoding after it fails.
/// That failure is not one a statement reaches through its depth — the host's entry
/// check refuses a catalog the thread could not encode before the statement runs, and
/// an expression the statement adds is encoded where it is added
/// (`pglet_sql::depth::check_stored_expr`) — which leaves only a failure of the
/// encoder itself.
///
/// # Errors
///
/// The encoding's error, after the staged writes are discarded.
pub fn autocommit_publish_bytes(
    conn_id: i32,
    schema: &Schema,
    storage: &mut StorageBackend,
) -> Result<Vec<u8>, Error> {
    catalog_bytes(schema, storage).inspect_err(|_| {
        let _ = storage.rollback(conn_id);
    })
}

/// Whether a plan that failed with `error` against another connection's uncommitted
/// catalog change failed **because of** that change.
///
/// `at_committed` is the same statement planned against the catalog committed when that
/// connection took the writer ([`TransactionSession::catalog_at_writer`]).
///
/// A plan that succeeds there, or fails there with another error, failed on the change:
/// on 18.6 the statement's parse analysis waits on the lock that change holds, so a host
/// that can wait waits for the block to end and plans again, and one that cannot refuses
/// it. A plan that fails there with the same error failed for its own reason — a column
/// no catalog has, a cast no catalog decides, a syntax error — and that error is its
/// answer at once, as 18.6's is where the lock it takes, if any, is not the block's. No
/// error is exempted by its SQLSTATE: pglet's parse and analysis are one pass, and some
/// of its `42601`s are analysis's, decided by the catalog (a set operation's column
/// counts, where a side is a `SELECT *`), so the plan against the committed catalog
/// decides a syntax error as it decides any other. That one pass also decides which
/// failure is first: a syntax error the pass reaches only after a check the block's
/// change decides — one the two catalogs fail differently, in code or message, or only
/// one fails, as `SELECT * FROM t UNION ALL SELECT 7, 8, 9 WHERE`'s column counts behind
/// a dropped column, checked before the `WHERE` is read — is the writer's, where 18.6's
/// raw parse answers it at once (the repo's `EDGES.md` row 238); behind a check both
/// catalogs fail alike, the check's error is the answer, at once.
#[must_use]
pub fn plan_failure_is_the_writers<T>(error: &Error, at_committed: &Result<T, Error>) -> bool {
    match at_committed {
        Ok(_) => true,
        Err(again) => {
            again.sqlstate() != error.sqlstate() || again.to_string() != error.to_string()
        }
    }
}

/// One level of the transaction stack: the block's outermost frame (`BEGIN`) or a
/// savepoint. See the module docs for what each field undoes.
#[derive(Debug)]
struct Frame {
    /// The savepoint's name, or `None` for the block's outermost frame.
    ///
    /// Only a named frame can be a `ROLLBACK TO` / `RELEASE` target, which is what
    /// keeps either of them from unwinding the whole block: `frames[0]` has no name
    /// and no lookup reaches it.
    name: Option<String>,
    /// The catalog as it stood when this frame opened — shared, not copied, by the frames
    /// a re-base sets to one committed catalog, and by what the host tells other
    /// connections about the block ([`TransactionSession::catalog_at_writer`]).
    schema: Arc<Schema>,
    /// The whole `MemoryStorage` as it stood when this frame opened; `None` on the
    /// B+tree backend, which buffers its own pending writes — **and `None` on every
    /// savepoint frame regardless of backend**, because a savepoint's data restore
    /// is the storage backend's own stack (`StorageBackend::savepoint`), which the
    /// memory backend implements with the same whole-store clone.
    storage: Option<StoreSnapshot>,
    /// The length of [`TransactionSession::sequence_swaps`] when this frame opened —
    /// the point the log is read from, and truncated back to, for this frame.
    sequence_swaps_at_open: usize,
}

/// What an aborted transaction block does with the statement in front of it.
///
/// Three answers rather than a boolean, because the middle one is neither running
/// the statement nor refusing it: Postgres converts a `COMMIT`/`END` in an aborted
/// block into a rollback and reports command tag `ROLLBACK`, which connection
/// poolers and ORMs that unconditionally commit at block end rely on to recover the
/// session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AbortedBlockAction {
    /// Run the statement: the block is healthy, or this statement is an exit
    /// statement the aborted block still admits (`ROLLBACK` and its spellings,
    /// which execute normally and end the block through their command tag).
    Run,
    /// End the block and answer command tag `ROLLBACK` without parsing or executing
    /// anything — Postgres's implicit rollback for `COMMIT` / `END`.
    ImplicitRollback,
    /// Answer [`Error::AbortedTransaction`] (`25P02`).
    Refuse,
}

/// A session's transaction block: its status, the frames a rollback unwinds, and
/// whether it has taken the writer.
///
/// One session belongs to one logical connection. [`TransactionSession`] records
/// which `conn_id` opened the block: [`Self::record_statement`], [`Self::fail`],
/// [`Self::take_writer`] and the aborted-block gate ignore a statement issued on any
/// other, and [`Self::savepoint`], [`Self::rollback_to_savepoint`] and
/// [`Self::release_savepoint`] answer it `25P01`, as with no block open. A single
/// [`crate::Database`] can route several logical connections through `execute_as`, and
/// an autocommit statement on a second `conn_id` must not accumulate into the first's
/// frame. [`Self::begin`], [`Self::commit`] and [`Self::rollback`] do not ask: the host
/// refuses another connection's transaction control while a block is open
/// (`crate::Database`'s `55P03`), so they reach here only for the block's own.
#[derive(Debug)]
pub struct TransactionSession {
    status: TransactionStatus,
    /// The connection that opened the current block. Meaningless while
    /// `status` is [`TransactionStatus::Idle`].
    conn_id: i32,
    /// The block's levels, outermost first: `frames[0]` is `BEGIN`, and one more
    /// per open savepoint.
    frames: Vec<Frame>,
    /// Whether the block has taken the writer ([`Self::take_writer`]) — which is what
    /// every rollback of it restores on, and what a block that never wrote skips.
    holds_writer: bool,
    /// Append-only log of the sequence **counter-cell swaps** this block has done:
    /// `(name, the last_value the outgoing cell held)`, in statement order. See
    /// [`Schema::restore_from`] for what a rollback does with it, and
    /// [`Self::record_statement`] for what puts entries here.
    ///
    /// A log rather than a per-frame map, and the reason is the interaction: a
    /// `ROLLBACK TO` truncates it back to the target frame's baseline, which undoes
    /// the swaps recorded after that frame opened for **every** enclosing frame at
    /// once and leaves the earlier ones standing — which is exactly right, since a
    /// swap the rollback undid no longer separates any frame from the current cell.
    /// Per-frame maps cannot express that without timestamping their entries.
    sequence_swaps: Vec<(String, i64)>,
    /// Every sequence whose counter **cell** a statement of this block replaced, as
    /// [`Self::record_statement`] records them, a `ROLLBACK TO SAVEPOINT` since
    /// notwithstanding — the ones whose counter a durable
    /// catalog is compared on ([`DurableCatalog::differs`]). Unlike `sequence_swaps` it
    /// is never truncated: a cell a `ROLLBACK TO` put back may still sit replaced in a
    /// durable catalog a save wrote before it. A list, each name once: a block replaces
    /// a handful of cells, and a set's code costs the wasm build more than a scan costs
    /// a block.
    replaced_cells: Vec<String>,
    /// The catalog the store's durable state held at `BEGIN`, read again when the block
    /// takes the writer ([`StorageBackend::persisted_catalog`]), held so that an identical read later
    /// proves no save wrote a catalog since — and so the block's catalog changes are in
    /// no durable state, and nothing needs comparing. `None` for a store with no durable
    /// state, and where the read failed.
    durable_catalog_at_begin: Option<Arc<Vec<u8>>>,
    /// The store's [`StorageBackend::commit_stamp`] when the block began — the state the
    /// storage snapshot its reads see holds.
    committed_at_begin: u64,
    /// The stamp of the state the block's snapshot holds, set by its first statement that
    /// takes one ([`Self::note_snapshot`]) — `None` before it — which a `REPEATABLE READ`
    /// block's first write asks whether anything committed past ([`Self::take_writer`]).
    snapshot_stamp: Option<u64>,
    /// The catalog generation when the block took the writer: a catalog at any other
    /// holds a change the block made ([`Self::catalog_at_anothers_writer`]).
    generation_at_writer: u64,
    /// Whether the host has held the catalog mutably while the block was open
    /// ([`Self::lend_catalog`]) — so a frame's stamp equal to the live one no longer
    /// shows that nothing a parse reads changed since the frame opened.
    catalog_lent: bool,
}

impl Default for TransactionSession {
    fn default() -> Self {
        Self::new()
    }
}

impl TransactionSession {
    /// A session with no transaction open.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            status: TransactionStatus::Idle,
            conn_id: 0,
            frames: Vec::new(),
            holds_writer: false,
            sequence_swaps: Vec::new(),
            replaced_cells: Vec::new(),
            durable_catalog_at_begin: None,
            committed_at_begin: 0,
            snapshot_stamp: None,
            generation_at_writer: 0,
            catalog_lent: false,
        }
    }

    /// The block's status, for the wire's `ReadyForQuery` and for a host's own
    /// bookkeeping.
    #[must_use]
    pub const fn status(&self) -> TransactionStatus {
        self.status
    }

    /// How many frames the block holds — `0` when idle, `1` for a plain
    /// `BEGIN … COMMIT`, and one more per open savepoint.
    ///
    /// Exposed because it is the observable for "the rollback state was released":
    /// a connection that disconnects mid-transaction must leave none behind, or its
    /// pinned MVCC snapshot keeps `min_active_version` from advancing forever.
    #[must_use]
    pub const fn frame_depth(&self) -> usize {
        self.frames.len()
    }

    /// Whether the open block has taken the writer — `false` while idle.
    #[must_use]
    pub const fn holds_writer(&self) -> bool {
        self.holds_writer
    }

    /// The connection whose block is open (healthy or aborted), or `None` while idle.
    #[must_use]
    pub const fn owner(&self) -> Option<i32> {
        if matches!(self.status, TransactionStatus::Idle) {
            None
        } else {
            Some(self.conn_id)
        }
    }

    /// Whether a block is open (healthy or aborted) on `conn_id`.
    ///
    /// Every state-accumulating operation is gated on this so a statement issued on
    /// a *different* logical connection of the same `Database` neither joins this
    /// frame nor reads its aborted state.
    #[must_use]
    pub const fn owns(&self, conn_id: i32) -> bool {
        !matches!(self.status, TransactionStatus::Idle) && self.conn_id == conn_id
    }

    /// What `conn_id` should do with `sql` given the block's state — the gate every
    /// phase reads.
    ///
    /// A healthy or absent block always answers [`AbortedBlockAction::Run`]; only an
    /// aborted one classifies, and it classifies through the lexer (see
    /// [`classify_transaction_exit`]).
    #[must_use]
    pub fn aborted_block_action(&self, conn_id: i32, sql: &str) -> AbortedBlockAction {
        if !(self.owns(conn_id) && self.status == TransactionStatus::Failed) {
            return AbortedBlockAction::Run;
        }
        match classify_transaction_exit(sql) {
            Some(ExitStatement::ImplicitRollback) => AbortedBlockAction::ImplicitRollback,
            Some(ExitStatement::Rollback) => AbortedBlockAction::Run,
            None => AbortedBlockAction::Refuse,
        }
    }

    /// Open a transaction for `conn_id`, snapshotting what a rollback will restore.
    ///
    /// A `BEGIN` while a block is already open — [`TransactionStatus::InTransaction`]
    /// **or** the aborted `Failed` — is a **no-op**, matching Postgres's effect (it
    /// emits `WARNING: there is already a transaction in progress` and changes
    /// nothing; pglet wires no notices). The early return is not decoration: the
    /// storage `begin` is silently idempotent, so re-snapshotting would keep the
    /// *first* storage state beside a *second* catalog one, and the eventual
    /// `ROLLBACK` would restore storage to before an in-transaction DDL and the
    /// catalog to after it — the durable phantom the repo's `docs/ddl.md` describes.
    ///
    /// # Errors
    ///
    /// Returns the storage error if the backend can't start the transaction; the
    /// session is left `Idle` with no frame, so nothing claims a block that never
    /// opened.
    pub fn begin(
        &mut self,
        conn_id: i32,
        schema: &Schema,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        if !matches!(self.status, TransactionStatus::Idle) {
            return Ok(());
        }
        storage.begin(conn_id)?;
        self.status = TransactionStatus::InTransaction;
        self.conn_id = conn_id;
        self.holds_writer = false;
        self.sequence_swaps.clear();
        self.replaced_cells.clear();
        self.durable_catalog_at_begin = storage.persisted_catalog().ok().flatten();
        self.committed_at_begin = storage.commit_stamp();
        self.snapshot_stamp = None;
        self.catalog_lent = false;
        self.frames.push(Frame {
            name: None,
            schema: Arc::new(schema.clone()),
            storage: store_snapshot(storage),
            sequence_swaps_at_open: 0,
        });
        Ok(())
    }

    /// `conn_id`'s block is about to run a statement that takes a snapshot, before it waits
    /// for anything.
    ///
    /// On 18.6 that is every statement but transaction control, `LOCK`, `SET`, `SHOW` and a
    /// few pglet does not plan (`PlannedStmtRequiresSnapshot`); here it is every planned
    /// statement but transaction control, and a `COPY` — not `DEALLOCATE` or `DISCARD`,
    /// which 18.6 counts and pglet answers as session commands (the repo's `EDGES.md` row
    /// 237). The block's first such statement fixes its isolation level (`settings`, 18.6's
    /// `FirstSnapshotSet`), and says what state a `REPEATABLE READ` block's first write is
    /// checked against ([`Self::take_writer`]): `first_write_at`, the store's
    /// [`StorageBackend::commit_stamp`] read at the statement's start, where the statement
    /// is the block's first write — it reads nothing before its re-base — and otherwise the
    /// stamp at `BEGIN`, since the storage snapshot the block reads is `BEGIN`'s. A no-op on
    /// another connection's block and after the block's first such statement.
    pub const fn note_snapshot(
        &mut self,
        conn_id: i32,
        settings: &mut SessionSettings,
        first_write_at: Option<u64>,
    ) {
        if !self.owns(conn_id) || self.snapshot_stamp.is_some() {
            return;
        }
        settings.note_block_statement();
        self.snapshot_stamp = Some(match first_write_at {
            Some(stamp) if !self.holds_writer => stamp,
            _ => self.committed_at_begin,
        });
    }

    /// Whether `conn_id`'s open block has run a statement that takes a snapshot
    /// ([`Self::note_snapshot`]) — `true` for another connection's block and with none
    /// open, where nothing is left to note.
    #[must_use]
    pub const fn snapshot_taken(&self, conn_id: i32) -> bool {
        !self.owns(conn_id) || self.snapshot_stamp.is_some()
    }

    /// Record that `conn_id`'s block takes the writer for a statement's write, **re-basing
    /// every frame** on the state committed now: the storage snapshot, the directory and
    /// table-id counter each frame restores (`StorageBackend::rebase`), each frame's
    /// catalog clone, the memory store's clone, and `durable_catalog_at_begin`.
    ///
    /// Called by the host **before** the block's first write stages anything — the
    /// re-base is sound only because nothing is staged — and with the host's exclusion
    /// already in place: the native server calls it holding the writer lock, the embed
    /// facade having refused every other connection's write. `schema` and `storage` are
    /// then the committed state. A no-op on another connection's block, with no block
    /// open, and once the block holds the writer.
    ///
    /// Why: a frame taken at `BEGIN` or at a savepoint before the first write is older
    /// than what other connections committed since, so restoring it would take their
    /// commits back, and a write decided against its storage snapshot would miss them —
    /// a key committed since, a parent deleted since, a value updated since.
    ///
    /// **A block whose level keeps one snapshot (`uses_transaction_snapshot`: `REPEATABLE
    /// READ`, `SERIALIZABLE`) keeps it**: where the store moved past the stamp its snapshot
    /// holds ([`Self::note_snapshot`] — this statement's start where it is the block's first
    /// statement that takes a snapshot; `BEGIN`'s, which the block's reads see, where an
    /// earlier statement took one — a `SET`, `SHOW`, `SAVEPOINT`, `DEALLOCATE` or `DISCARD`
    /// takes none), its write is refused `40001` and the block takes nothing; where it did
    /// not, the re-base takes in only what committed before that stamp — where this is the
    /// first statement that takes one, what committed between `BEGIN` and it, which 18.6's
    /// snapshot, taken at that statement, sees too. That is coarser than 18.6, which refuses only a write
    /// meeting a row changed since the block's snapshot: any move of the stamp refuses it
    /// here (the repo's `EDGES.md` row 232).
    ///
    /// # Errors
    ///
    /// [`Error::SerializationFailure`] (`40001`) as above; the block then does not hold
    /// the writer.
    pub fn take_writer(
        &mut self,
        conn_id: i32,
        uses_transaction_snapshot: bool,
        schema: &Schema,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        if !self.owns(conn_id) || self.holds_writer {
            return Ok(());
        }
        let snapshot_at = self.snapshot_stamp.unwrap_or(self.committed_at_begin);
        if uses_transaction_snapshot && storage.commit_stamp() != snapshot_at {
            return Err(Error::SerializationFailure);
        }
        self.rebase_on_committed(schema, storage, conn_id);
        Ok(())
    }

    /// [`Self::take_writer`] for a host's own change to an open block — through
    /// `crate::Database::schema_mut`, `storage_mut` or `reattach_persisted_indexes`,
    /// which cannot answer an error — re-basing whatever the block's level: a host's
    /// change is not a statement a serialization failure could be answered to.
    pub fn take_writer_for_host(
        &mut self,
        conn_id: i32,
        schema: &Schema,
        storage: &mut StorageBackend,
    ) {
        if !self.owns(conn_id) || self.holds_writer {
            return;
        }
        self.rebase_on_committed(schema, storage, conn_id);
    }

    /// [`Self::take_writer`]'s re-base, and the record that the block holds the writer.
    fn rebase_on_committed(&mut self, schema: &Schema, storage: &mut StorageBackend, conn_id: i32) {
        storage.rebase(conn_id);
        let committed = Arc::new(schema.clone());
        for frame in &mut self.frames {
            frame.schema = Arc::clone(&committed);
        }
        if let Some(outer) = self.frames.first_mut()
            && outer.storage.is_some()
        {
            outer.storage = store_snapshot(storage);
        }
        self.durable_catalog_at_begin = storage.persisted_catalog().ok().flatten();
        self.generation_at_writer = schema.generation;
        self.holds_writer = true;
    }

    /// The catalog committed when the open block took the writer — what its frames were
    /// re-based on — or `None` while it has not taken it. The native server hands it to
    /// the writer lock (`pglet_server`'s `WriterLock::note_grant`), so another connection
    /// can plan against it ([`plan_failure_is_the_writers`]).
    #[must_use]
    pub fn catalog_at_writer(&self) -> Option<&Arc<Schema>> {
        if self.holds_writer {
            self.frames.first().map(|frame| &frame.schema)
        } else {
            None
        }
    }

    /// [`Self::catalog_at_writer`], asked by `conn_id` — another connection than the
    /// block's — about a catalog `schema` at a generation other than the one the block
    /// took the writer at, so holding a change the block made and has not ended: the
    /// catalog a plan of `conn_id`'s that failed against `schema` is planned against
    /// again. `None` otherwise.
    #[must_use]
    pub fn catalog_at_anothers_writer(
        &self,
        conn_id: i32,
        schema: &Schema,
    ) -> Option<&Arc<Schema>> {
        if self.owner().is_some_and(|owner| owner != conn_id)
            && schema.generation != self.generation_at_writer
        {
            self.catalog_at_writer()
        } else {
            None
        }
    }

    /// Commit the block: publish the staged writes, then persist the catalog beside
    /// them.
    ///
    /// **A commit of an *aborted* block rolls it back instead**, publishing nothing —
    /// Postgres's implicit rollback, reached here rather than only through the SQL
    /// `COMMIT` because a host can commit programmatically ([`crate::Database::commit`],
    /// which the embed API and the wasm bindings expose) and the two entry points must
    /// not disagree about what a `COMMIT` does. Without it the SQL path converted while
    /// the programmatic one published the partial effects of the statement that failed
    /// — a half-applied `UPDATE` among the measured shapes.
    ///
    /// The catalog is encoded **before** the storage commit publishes anything
    /// ([`catalog_bytes`]), and a failure there **rolls the block back and ends it**:
    /// a `COMMIT` that fails is a rollback on 18.6 — measured with a deferred foreign
    /// key violated at `COMMIT`, which answers the error, leaves none of the block's
    /// rows, and returns the connection to `Idle`. Left open, the block went aborted
    /// and waited on a `ROLLBACK` 18.6 does not ask for. A failure of the storage
    /// commit itself, or of the durable meta write after it, is an I/O failure past
    /// the point the block's writes were handed to the store, and returns with the
    /// block as the store left it — except a write the device refused before anything
    /// was published (`Error::WriteRefused`), which drops the staged writes and ends the
    /// block rolled back: its catalog restored, or, where the rollback cannot restore it
    /// or cannot save what it restored, the store failed (`# Errors`).
    ///
    /// **A refusal that fails the store answers "outcome unknown" once a save may have
    /// made the block's catalog changes durable** — for a healthy block and an aborted
    /// one alike, since the client asked to commit either way. Every save but a
    /// rollback's counter save encodes the live catalog, which holds an open block's DDL,
    /// and that one writes nothing while the live catalog differs from the durable one in
    /// anything but counters (`TransactionSession::counter_save`) — so a save after a
    /// catalog change of the block's — a host's `sync` — made that change durable, and a
    /// refused `COMMIT` that fails the store leaves it there: the
    /// rollback's save, which would take it back, is refused too under a fault that
    /// persists, and a block that created or dropped a table or an index fails the store
    /// without one.
    /// The block's rows are not durable and its DDL is, so `Error::DurableWriteFailed` —
    /// the statement did not persist — would be false; the answer is
    /// `Error::DurableWriteUnconfirmed`. Which blocks: one that took the writer, and
    /// whose durable catalog — written by a save since it did — differs from the one it
    /// was re-based on then but for counters advanced in place
    /// (`TransactionSession::durable_catalog_holds_block`). A healthy block's `COMMIT` asks
    /// it only once a refusal has come, so one that succeeds pays nothing for the
    /// question; an aborted block's asks it before its rollback runs, whatever that
    /// rollback answers. Either asks it of the durable catalog as it stood before the step
    /// that failed: before the rollback, which
    /// may save, and — for a refusal of the save itself — before that save started, since a
    /// save refused past its catalog batch has already replaced what it would read with the
    /// catalog it was writing, which no durable slot names. It is read off the catalogs, not
    /// off which statements ran, so a change a host made through `crate::Database::schema_mut`, or
    /// DDL a `ROLLBACK TO SAVEPOINT` undid after a save wrote it, counts, and DDL undone
    /// before any save does not. Any other block keeps the
    /// definite answer: no durable state holds any of it. A host's aborted-block `COMMIT`, which
    /// 18.6 converts to a rollback, comes here rather than to
    /// [`TransactionSession::rollback`] for this answer; a `ROLLBACK` refused before its
    /// save's slot write keeps the definite one, since what it says did not persist — the
    /// undo — did not.
    ///
    /// # Errors
    ///
    /// Returns the encoding error (the block rolled back), or the storage commit /
    /// state-persist error, without having reported success — `Error::WriteRefused`
    /// with the block ended rolled back, when the device refused a write the block's
    /// commit could drop, and `Error::DurableWriteFailed` with the store failed when
    /// the rollback that follows could not take the block's catalog changes back — or,
    /// for an aborted block, whatever [`TransactionSession::rollback`] returns; in either
    /// case `Error::DurableWriteUnconfirmed` in place of `Error::DurableWriteFailed` past
    /// a save that may have made the block's catalog changes durable (above).
    pub fn commit(
        &mut self,
        conn_id: i32,
        schema: &mut Schema,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        if self.status != TransactionStatus::Failed {
            return self.commit_healthy(conn_id, schema, storage);
        }
        // Read before the rollback saves or ends the block, either of which moves or
        // clears what this reads.
        let catalog_may_be_durable =
            self.durable_catalog_holds_block(&storage.persisted_catalog(), None);
        self.rollback(conn_id, schema, storage).map_err(|e| {
            if catalog_may_be_durable {
                outcome_unknown(e)
            } else {
                e
            }
        })
    }

    /// Whether the catalog the store's durable state holds may hold some of this block's
    /// catalog changes: the block took the writer, a save has written a catalog since
    /// it did, and the durable catalog differs from the one the block was re-based on
    /// ([`DurableCatalog::differs`]) — which is what a save after a change the block made
    /// leaves, whichever door made the change — and, given `restored`, from that one too,
    /// whose encoding is then left in the slot beside it for the save that follows.
    ///
    /// The comparison is skipped where its answer is known: `false` on a backend with no
    /// durable state, and where the durable catalog is the
    /// very one read when the block took the writer (`durable_catalog_at_begin`), since
    /// no save has then written any change of the block's. `true` where the durable catalog cannot be
    /// read, the cautious answer.
    fn durable_catalog_holds_block(
        &self,
        durable: &pglet_storage::Result<Option<Arc<Vec<u8>>>>,
        restored: Option<(&Schema, &mut Option<Vec<u8>>)>,
    ) -> bool {
        if !self.holds_writer {
            return false;
        }
        let Some(frame) = self.frames.first() else {
            return false;
        };
        let bytes = match durable {
            Ok(Some(bytes)) => bytes,
            Ok(None) => return false,
            Err(_) => return true,
        };
        if self
            .durable_catalog_at_begin
            .as_ref()
            .is_some_and(|at_begin| Arc::ptr_eq(at_begin, bytes))
        {
            return false;
        }
        let mut durable = DurableCatalog::new(bytes);
        let Ok(begin_encoded) = serialize_schema(&frame.schema) else {
            return true;
        };
        if !durable.differs(&frame.schema, &begin_encoded, &self.replaced_cells) {
            return false;
        }
        let Some((restored, slot)) = restored else {
            return true;
        };
        let Ok(restored_encoded) = serialize_schema(restored) else {
            return true;
        };
        let differs = durable.differs(restored, &restored_encoded, &self.replaced_cells);
        *slot = Some(restored_encoded);
        differs
    }

    /// The catalog a rollback saves for the counters it leaves ahead of the durable ones —
    /// the durable catalog `held`, with each sequence's counter raised to the one `live`
    /// holds under the same name where that is higher — or `None` when there is nothing to
    /// write, or when it must not be written (below).
    ///
    /// **Only counters move.** Everything else is `held`'s: none of the rolled-back block's
    /// own DDL reaches the durable state through this save. A counter is only ever raised,
    /// so a live one below the durable one — a `setval` moved back — is not written.
    ///
    /// **A counter is identified by its sequence's name.** The catalog holds no identity
    /// for a counter cell but its name, so the live cell under a name is taken as the
    /// durable one's successor. Where the name held a different cell live than durably,
    /// raising would be the safe direction: a durable counter raised past the values a cell
    /// handed out hands out none of them again, where one left behind would — at the cost
    /// of a reopen that skips ahead.
    ///
    /// **Whether to write at all.** With no save since the block took the writer (the
    /// durable catalog is the very one read then), only a counter past the one the re-base
    /// found is anything the block drew. A durable counter behind the live one at the
    /// re-base is something else: a failed auto-commit statement's draw, which no
    /// connection was handed — or a value another block handed out before this block took
    /// the writer and whose own
    /// rollback wrote nothing, the structure check below having skipped it, which a reopen
    /// hands out again until a save writes it (the repo's `EDGES.md` row 214). This rollback
    /// does not tell the two apart, and writing for the first alone would fail a store under
    /// a full disk for nothing. So such a rollback writes only where some counter the
    /// re-base found stands past it, and then raises every counter the live catalog holds
    /// ahead of the durable one, as any save would — a lag of either kind included. Past a
    /// save since the re-base, the durable counter is compared directly.
    ///
    /// "Past" is "ahead" only because every draw is `+1`: pglet parses and ignores
    /// `INCREMENT BY`, `MINVALUE` / `MAXVALUE` and `CYCLE` (`EDGES.md` row 215), so a counter
    /// moves up and never wraps. Honouring them moves a descending draw below the durable
    /// counter and a wrap back down, and this save would keep neither.
    ///
    /// **Not written where the live catalog differs from `held` in anything but counters.**
    /// A save writes the store's current tree beside the catalog, and a structure the live
    /// catalog holds and no save has written — a change a host made through
    /// `crate::Database::schema_mut` outside any block — may have a footprint in that
    /// tree the older catalog does not account for. So the counters wait for the next save that writes the live
    /// catalog, and a reopen before it hands their values out again (row 214). The check
    /// sees only what a catalog names: an index a block built and dropped leaves its entries
    /// with no catalog naming it, and this save writes them as any save does. One footprint
    /// measured to read wrong — index entries whose number a reopen handed the next index —
    /// reads nothing: a reopen seeds its OID counter past every index number the tree
    /// holds an entry under (`crate::Database::reattach_persisted_indexes`). Another was
    /// measured, with the check removed, through another connection's open block — a route
    /// the writer keeps closed, since that block holds the writer and a rollback's draw
    /// waits for it: a block that drops an enum and creates it again with its labels
    /// reordered rebuilds a unique partial index whose predicate reads it, in the tree at
    /// once, and the durable catalog's order written beside those entries put a committed
    /// row back inside the predicate with no entry for it — a reopen accepted a duplicate
    /// of the row under the unique index.
    ///
    /// The durable catalog is decoded, except where no save came since the re-base and the
    /// catalog the re-base found encodes to it byte for byte, where that catalog is taken in
    /// its place: an encoding costs about a quarter of a decoding (measured at 2 000
    /// tables). So this is the last reader of the block's outer frame, which `Self::end`
    /// then clears.
    ///
    /// # Errors
    ///
    /// The durable catalog's decoding error, or an encoding error.
    fn counter_save(
        &mut self,
        live: &Schema,
        held: &Arc<Vec<u8>>,
    ) -> Result<Option<Vec<u8>>, Error> {
        let Some(frame) = self.frames.first_mut() else {
            return Ok(None);
        };
        let saved_since = !self
            .durable_catalog_at_begin
            .as_ref()
            .is_some_and(|at_begin| Arc::ptr_eq(at_begin, held));
        // with no save since the re-base, the block drew nothing unless a counter the
        // re-base found stands past it — and the common rollback decodes nothing
        if !saved_since
            && !frame.schema.sequences.iter().any(|(name, at_begin)| {
                live.sequences
                    .get(name)
                    .is_some_and(|now| now.last_value > at_begin.last_value)
            })
        {
            return Ok(None);
        }
        if held.is_empty() {
            return Ok(None);
        }
        let mut durable = if !saved_since && serialize_schema(&frame.schema)? == **held {
            Arc::unwrap_or_clone(std::mem::take(&mut frame.schema))
        } else {
            deserialize_schema(held)?
        };
        // Set every durable counter to the live one of its name, keeping what each held:
        // the ones the save leaves where they were are put back once the check has run.
        let mut raised_any = false;
        let mut kept = Vec::new();
        for (index, (name, sequence)) in durable.sequences.iter_mut().enumerate() {
            let Some(now) = live.sequences.get(name) else {
                continue;
            };
            if now.last_value > sequence.last_value {
                raised_any = true;
            } else if now.last_value != sequence.last_value {
                kept.push((index, sequence.last_value));
            }
            sequence.last_value = now.last_value;
        }
        if !raised_any {
            return Ok(None);
        }
        // The durable catalog with the live counters, which is the live catalog exactly
        // when the two differ in nothing else.
        let live_encoded = serialize_schema(live)?;
        if serialize_schema(&durable)? != live_encoded {
            return Ok(None);
        }
        if kept.is_empty() {
            return Ok(Some(live_encoded));
        }
        let mut sequences = durable.sequences.values_mut().enumerate();
        for (index, last_value) in kept {
            if let Some((_, sequence)) = sequences.find(|(i, _)| *i == index) {
                sequence.last_value = last_value;
            }
        }
        serialize_schema(&durable).map(Some)
    }

    /// [`Self::commit`] of a healthy block, answering as the store did.
    ///
    /// A block that never took the writer staged nothing and changed no catalog, so its
    /// commit publishes and **saves** nothing — a save would encode the live catalog,
    /// which holds whatever the writer's block has done so far.
    fn commit_healthy(
        &mut self,
        conn_id: i32,
        schema: &mut Schema,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        if !self.holds_writer {
            storage.end_unwritten(conn_id);
            self.end(conn_id, storage);
            return Ok(());
        }
        let schema_bytes = match catalog_bytes(schema, storage) {
            Ok(bytes) => bytes,
            Err(e) => {
                // The COMMIT's own error is what the client is told; a rollback that
                // also fails (its save) must not replace it.
                let _ = self.rollback(conn_id, schema, storage);
                return Err(e);
            }
        };
        // A write the device refuses before anything is published drops the block's
        // staged writes (storage decides whether it can: not past a published version,
        // and not when the block did DDL the storage layer cannot take back once its
        // commit has consumed the transaction). The block then ends rolled back, as
        // 18.6's does when its commit raises an `ERROR` — the rollback takes back its
        // catalog changes too. A rollback that fails (its storage restore or its save)
        // leaves the catalog holding what no durable state does, and the store fails.
        //
        // What a failure answers is decided where it fails, never ahead of it: a commit
        // that succeeds pays nothing for the comparison behind that answer
        // (`Self::durable_catalog_holds_block`). Each read comes before whatever moves or
        // clears what it reads — the rollback, the save's catalog write — and the refused
        // commit itself writes no catalog.
        if let Err(e) = storage.commit_discardable(conn_id) {
            let e = Error::from(e);
            let catalog_may_be_durable = matches!(
                e,
                Error::WriteRefused { .. } | Error::DurableWriteFailed { .. }
            ) && self
                .durable_catalog_holds_block(&storage.persisted_catalog(), None);
            if let Error::WriteRefused { reason, disk_full } = &e
                && self
                    .rollback_saving(conn_id, schema, storage, false)
                    .is_err()
            {
                if let Some(failure) = storage.write_failure() {
                    failure.record(reason.clone());
                }
                let failed = Error::DurableWriteFailed {
                    reason: reason.clone(),
                    disk_full: *disk_full,
                };
                return Err(if catalog_may_be_durable {
                    outcome_unknown(failed)
                } else {
                    failed
                });
            }
            return Err(if catalog_may_be_durable {
                outcome_unknown(e)
            } else {
                e
            });
        }
        let durable = storage.persisted_catalog();
        if let Err(e) = storage.save_state(schema_bytes) {
            let e = Error::from(e);
            let catalog_may_be_durable = matches!(e, Error::DurableWriteFailed { .. })
                && self.durable_catalog_holds_block(&durable, None);
            return Err(if catalog_may_be_durable {
                outcome_unknown(e)
            } else {
                e
            });
        }
        self.end(conn_id, storage);
        Ok(())
    }

    /// Roll the block back: discard the staged writes and restore the catalog.
    ///
    /// **A block that took the writer ([`Self::take_writer`]) restores unconditionally**:
    /// the storage backend's own frame (`StorageBackend::rollback` — the B+tree's pending
    /// writes, directory and table-id counter; nothing on memory), then the catalog from
    /// the block's outermost frame, and on memory storage the whole store from the frame's
    /// clone. Every frame was re-based when the block took the writer, and nothing but
    /// this block has changed the catalog or the store since, so the restore is exact.
    /// **A block that never took the writer restores nothing**: it staged nothing and
    /// changed no catalog, and its frames are older than what other connections
    /// committed while it read, which a restore would take back — so it ends, saving
    /// nothing (`StorageBackend::end_unwritten`).
    ///
    /// The restore keeps `generation` where nothing moved it since the frame opened — a
    /// block that ran only DML keeps the plans cached against its catalog — and otherwise
    /// advances it past the live value instead of restoring the snapshot's lower one: the
    /// stamp must be **monotonic** so a prepared-plan cache keyed `plan_gen ==
    /// generation` can never reuse a value for two different catalogs — an ABA that let a
    /// cached plan probe a since-freed `index_oid` and fail loud with "index not found".
    /// A block that lent the host the catalog ([`Self::lend_catalog`]) always moves it.
    /// `next_oid` keeps the live high-water: a restored one would let a re-minted index
    /// reuse a freed storage `index_id` (`Schema::restore_from`).
    ///
    /// The block is always ended (status returns to `Idle`, frames cleared), even when a
    /// step fails, so the session stays usable.
    ///
    /// **A restored catalog is saved when the durable catalog holds a change the block
    /// made.** Every save but a counter save (below) encodes the live catalog, which
    /// holds an open block's DDL (§ Schema Snapshot Semantics in the repo's
    /// `docs/storage.md`), so a save during
    /// the block — a host's `sync` — makes this block's DDL durable, and a rollback that
    /// restored the catalog in memory alone would leave a reopen holding it: an index, a
    /// table, an enum's reversed members, a column whose existing rows read `NULL`. So
    /// when a save has written a catalog since the block took the writer, and the catalog
    /// the store's durable state holds differs from the one the block was re-based on
    /// **and** from the one just restored, both but for counters advanced in
    /// place (`TransactionSession::durable_catalog_holds_block`), the restored catalog
    /// is saved before the rollback returns. It is the catalogs that are compared, not a
    /// record of which statements ran, so a change a host made through
    /// `crate::Database::schema_mut` counts as DDL does. A save that wrote a catalog
    /// holding none of the block's changes — before its first one, or after a `ROLLBACK
    /// TO SAVEPOINT` undid them — leaves the rollback no restored catalog to save, nor does
    /// a durable catalog that already differed from the live one when the block took the
    /// writer with no save since, by a counter or a host's change no save had written,
    /// neither of them the block's; a
    /// refusal of a needless save would fail the store for nothing. The counter
    /// exception is `Schema::restore_from`'s: an in-place `nextval` or `setval` is no
    /// change a rollback takes back, so a durable counter behind the live one is no change
    /// of the block's to take back, while a sequence whose cell the block replaced by a
    /// route the restore takes back is compared whole (`DurableCatalog::differs` names the
    /// routes it does not). A store with no durable state, the in-memory B+tree among
    /// them, saves nothing. A refusal of that save fails the store, since the durable
    /// catalog then holds DDL no connection sees, and is the rollback's answer — the
    /// definite one for a refusal before the save's slot write, when none of the save is
    /// durable, and the unknown one at the slot write or its `fsync`, which may have
    /// landed. A store already failed is not saved to.
    ///
    /// **Otherwise a counter left ahead of the durable one is saved, and nothing else.** A
    /// `nextval` hands its value out as it runs and no rollback takes the advance back, so a
    /// durable counter the rollback left behind would let a reopen before the next save by
    /// anyone hand those values out again — after a clean close as after a crash. So on a
    /// store with durable state, where no restored catalog is saved, the rollback saves the
    /// **durable** catalog with each sequence's counter raised to the live one that stands
    /// past it (`TransactionSession::counter_save`, which says what "past" is and which
    /// counter is which): whatever sequence the block drew from, and nothing the live
    /// catalog holds beside the counters, so the rolled-back block's own DDL stays out of
    /// the durable state. It stands in for 18.6's `nextval`, which logs its
    /// advance ahead of use. Nothing is written where no counter is raised — a save since
    /// the advance may already hold it — nor where the live catalog differs from the durable
    /// one in anything but counters, whose next save the counters then wait for. Not by the
    /// rollback a refused `COMMIT` runs: the device has just refused a write, and a refused
    /// save would fail a store that `COMMIT` otherwise leaves serving. A refusal of this
    /// save fails the store too, and is answered as the restoring save's is. What a reopen
    /// can still hand out again is the repo's `EDGES.md` row 214.
    ///
    /// # Errors
    ///
    /// The storage restore's error (a page the store cannot read or write), then the
    /// error of the save that writes the restored catalog or the counters, or of
    /// building what it writes (the durable catalog's decoding, an encoding), or of the
    /// index-spec resync a restore that moved an enum runs.
    pub fn rollback(
        &mut self,
        conn_id: i32,
        schema: &mut Schema,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        self.rollback_saving(conn_id, schema, storage, true)
    }

    /// [`Self::rollback`], saving the counters it leaves ahead of the durable ones only when
    /// `save_counters` — which a `COMMIT` the device just refused passes `false`.
    fn rollback_saving(
        &mut self,
        conn_id: i32,
        schema: &mut Schema,
        storage: &mut StorageBackend,
        save_counters: bool,
    ) -> Result<(), Error> {
        if !self.holds_writer {
            storage.end_unwritten(conn_id);
            self.end(conn_id, storage);
            return Ok(());
        }
        // Discard pending writes + restore the storage allocators. Captured (not `?`) so
        // the block is still ended below — a wedged, still-open transaction on a failed
        // ROLLBACK is worse than a clean end plus the error.
        let storage_err = storage.rollback(conn_id).err();

        let catalog_err = self.frames.first().and_then(|outer| {
            let snapshot_schema = self.frame_catalog(outer, schema);
            let snapshot_storage = outer.storage.clone();
            // `Schema::restore_from` carries all three rollback rules — the monotonic
            // `generation`/`next_oid` stamps and the sequence counters — and is shared
            // with the savepoint rollback and the `DO` block's undo.
            restore_store_snapshot(storage, snapshot_storage);
            let moved = enums_moved_by_restore(schema, &snapshot_schema);
            schema.restore_from(snapshot_schema, &self.sequence_swaps);
            resync_index_specs(schema, storage, conn_id, &moved).err()
        });

        // What to save: the restored catalog, where the durable one holds a change the
        // block made and differs from the one just restored; else the durable catalog with
        // the counters the rollback leaves ahead of it (see the doc's paragraphs on each).
        // Read before `end` clears the frames and cells they read.
        let mut restored_encoded = None;
        let writable = storage.check_writable().is_ok();
        let durable = storage.persisted_catalog();
        let save_restored = writable
            && self.durable_catalog_holds_block(&durable, Some((schema, &mut restored_encoded)));
        let counters = match &durable {
            Ok(Some(held)) if save_counters && !save_restored && writable => {
                self.counter_save(schema, held)
            }
            _ => Ok(None),
        };
        self.end(conn_id, storage);

        let save_err = if save_restored {
            restored_encoded
                .map_or_else(|| catalog_bytes(schema, storage), Ok)
                .and_then(|bytes| storage.save_state(bytes).map_err(Error::from))
                .err()
        } else {
            counters
                .and_then(|bytes| {
                    bytes.map_or(Ok(()), |bytes| {
                        storage.save_state(bytes).map_err(Error::from)
                    })
                })
                .err()
        };

        // Surface the first failure (storage takes precedence); the block has been ended.
        if let Some(e) = storage_err {
            return Err(e.into());
        }
        if let Some(e) = catalog_err {
            return Err(e);
        }
        if let Some(e) = save_err {
            return Err(e);
        }
        Ok(())
    }

    /// Move a healthy block to the aborted state after a statement raised, and
    /// suspend the unique-key claims its staged writes hold against other
    /// connections' probes.
    ///
    /// A no-op outside a block, on another connection's statement, and on an already
    /// aborted block.
    ///
    /// **Why it takes storage.** `COMMIT`, `ROLLBACK` and a client disconnect drop the
    /// whole buffer, and `ROLLBACK TO SAVEPOINT` restores it from the savepoint's
    /// snapshot, so each of those releases exactly the claims whose writes ceased to
    /// exist. An abort ends nothing — the buffer stands, because a `ROLLBACK TO
    /// SAVEPOINT` can still hand it back — so the claims are suspended here, as 18.6's
    /// two arms release a failed transaction's locks: the block goes on claiming what its
    /// innermost open savepoint holds, and nothing with no savepoint open. No other
    /// connection's write meets the difference: the aborted block holds the writer until
    /// it ends, where 18.6 lets another connection past it at the error (the repo's
    /// `EDGES.md` row 230).
    pub fn fail(&mut self, conn_id: i32, storage: &mut StorageBackend) {
        if self.owns(conn_id) && matches!(self.status, TransactionStatus::InTransaction) {
            self.status = TransactionStatus::Failed;
            storage.suspend_claims(conn_id);
        }
    }

    /// Record the sequence counter cells one executed statement replaced.
    ///
    /// Two sources, each with the value the outgoing cell held: `sequence_swaps`, what
    /// `Plan::swapped_sequences` listed for the statement before it ran, and the cells
    /// `result` reports — `QueryResult::swapped_sequences`, which only a completed `DO`
    /// fills, since no plan can list its statements' replacements ahead. They are kept in
    /// statement order for the restore, and by name for the comparison a rollback and a
    /// refused `COMMIT` make against the durable catalog. Ignored on another connection's
    /// statement and with no block open.
    pub fn record_statement(
        &mut self,
        conn_id: i32,
        mut sequence_swaps: Vec<(String, i64)>,
        result: &Result<QueryResult, Error>,
    ) {
        if !self.owns(conn_id) {
            return;
        }
        if let Ok(done) = result {
            sequence_swaps.extend(done.swapped_sequences.iter().cloned());
        }
        for (name, _) in &sequence_swaps {
            if !self.replaced_cells.contains(name) {
                self.replaced_cells.push(name.clone());
            }
        }
        self.sequence_swaps.extend(sequence_swaps);
    }

    /// Apply the transaction lifecycle an executed statement calls for.
    ///
    /// The statement is **parsed and executed first** — a transaction-control plan
    /// executes to nothing but its command tag and this signal — and the signal is
    /// what opens or ends the block. So a statement reaches this only once the parser
    /// has accepted it.
    ///
    /// Deciding it from the SQL text instead, ahead of the parser, is what this
    /// replaced on the wasm leg, and it was wrong in both directions at once: a
    /// `starts_with` with no word boundary took `BEGINNING` / `COMMITTED` /
    /// `ROLLBACKX` into the lifecycle where 18.6 answers `42601`, half-honoured three
    /// statements 18.6 implements by dropping the part pglet does not model
    /// (`ROLLBACK TO SAVEPOINT sp` reported tag `ROLLBACK` **after discarding every
    /// write since `BEGIN`**), and missed `/* c */ BEGIN`, which returned tag `BEGIN`
    /// having opened nothing — so the write after it autocommitted and the `ROLLBACK`
    /// after that reverted nothing.
    ///
    /// **Dispatching on the command *tag* was the next version of the same mistake**,
    /// and it is why this takes a [`TransactionCommand`]: `ROLLBACK TO SAVEPOINT`
    /// reports tag `ROLLBACK`, exactly as 18.6 does, so a tag-keyed host reads it as
    /// a whole-block rollback and discards every write since `BEGIN` while reporting
    /// success. The tag is what the client is told; the signal is what happened.
    ///
    /// # Errors
    ///
    /// Returns the commit or rollback error, so a host surfaces it instead of
    /// reporting a commit that silently dropped the catalog, or a rollback whose
    /// restore or save failed.
    pub fn apply_command(
        &mut self,
        conn_id: i32,
        command: &TransactionCommand,
        schema: &mut Schema,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        match command {
            TransactionCommand::Begin(_) => self.begin(conn_id, schema, storage),
            TransactionCommand::Commit => self.commit(conn_id, schema, storage),
            TransactionCommand::Rollback => self.rollback(conn_id, schema, storage),
            TransactionCommand::Savepoint(name) => self.savepoint(conn_id, name, schema, storage),
            TransactionCommand::RollbackTo(name) => {
                self.rollback_to_savepoint(conn_id, name, schema, storage)
            }
            TransactionCommand::Release(name) => self.release_savepoint(conn_id, name, storage),
        }
    }

    /// Carry the outcome of one transaction command — `was` the block's status before it,
    /// `ok` whether it succeeded — to the session's settings, which are transactional as
    /// 18.6's GUCs are (`pglet_sql::catalog::SessionSettings`): a `BEGIN` opens their
    /// block at its isolation level, a savepoint saves them, a `ROLLBACK TO` puts back
    /// what its savepoint began with, a `RELEASE` keeps them, and a block's end keeps
    /// them only for a `COMMIT` of a healthy block that succeeded — a rollback of any
    /// other kind puts back what the block began with.
    ///
    /// Every host calls it wherever it applies a transaction command, so the settings'
    /// levels stand one per frame of this session.
    ///
    /// # Errors
    ///
    /// `25001` for a `BEGIN` inside an open block naming an isolation level the block
    /// can no longer take (18.6 applies the level as `SET TRANSACTION` does).
    pub fn carry_to_settings(
        &self,
        settings: &mut SessionSettings,
        command: &TransactionCommand,
        was: TransactionStatus,
        ok: bool,
    ) -> Result<(), Error> {
        if self.status == TransactionStatus::Idle {
            let committed = ok
                && was == TransactionStatus::InTransaction
                && *command == TransactionCommand::Commit;
            settings.end_block(committed);
            return Ok(());
        }
        match command {
            TransactionCommand::Begin(level) if ok => settings.begin_block(*level)?,
            TransactionCommand::Savepoint(_) if ok => settings.savepoint_block(),
            TransactionCommand::RollbackTo(_) if ok => {
                settings.rollback_to_block(self.frame_depth());
            }
            TransactionCommand::Release(_) if ok => settings.release_block(self.frame_depth()),
            _ => {}
        }
        Ok(())
    }

    /// Establish a savepoint named `name` in the open block.
    ///
    /// # Errors
    ///
    /// [`Error::NoActiveSqlTransaction`] (`25P01`) with no block open — Postgres's
    /// `CheckTransactionBlock`. An **aborted** block never reaches this: the gate
    /// ahead of the parser refuses `SAVEPOINT` with `25P02`, as 18.6 does. Also
    /// surfaces the backend's error if it cannot stage the level, in which case no
    /// frame is pushed and the two stacks stay in lockstep.
    pub fn savepoint(
        &mut self,
        conn_id: i32,
        name: &str,
        schema: &Schema,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        if !self.owns(conn_id) {
            return Err(Error::NoActiveSqlTransaction { stmt: "SAVEPOINT" });
        }
        self.check_lockstep(conn_id, storage)?;
        // Storage first: a failure there must leave no session frame behind, or the
        // next `ROLLBACK TO` restores the wrong backend level.
        storage.savepoint(conn_id)?;
        self.frames.push(Frame {
            name: Some(name.to_string()),
            schema: Arc::new(schema.clone()),
            storage: None,
            sequence_swaps_at_open: self.sequence_swaps.len(),
        });
        Ok(())
    }

    /// Discard everything written since the savepoint named `name`, **keeping** that
    /// savepoint and destroying every savepoint established after it.
    ///
    /// This is also the one statement an **aborted** block admits: a successful
    /// `ROLLBACK TO` clears the abort and the block continues, which is what makes a
    /// per-test harness able to recover from a failed statement without losing the
    /// whole block. A *failed* one (`3B001`) leaves the block aborted — that falls
    /// out of the ordinary rule that a raising statement aborts the block, not from
    /// anything here.
    ///
    /// In a block that took the writer it restores the savepoint's frame
    /// **unconditionally** — the storage backend's level (`StorageBackend::rollback_to_savepoint`)
    /// and the catalog — for the reason [`Self::rollback`] does; in one that never took
    /// it there is nothing to restore, and nothing is. It **saves nothing**: DDL it undid
    /// that a save during the block made durable stays on disk until the next save that
    /// encodes the live catalog — the block's own `sync` on a host that has one, the
    /// block's commit, or its rollback's, which compares the durable catalog with the one
    /// it restores rather than counting statements (the repo's `EDGES.md` row 211).
    ///
    /// # Errors
    ///
    /// [`Error::NoActiveSqlTransaction`] (`25P01`) with no block open;
    /// [`Error::SavepointNotFound`] (`3B001`) when the block holds no live savepoint
    /// of that name; or the backend's error.
    pub fn rollback_to_savepoint(
        &mut self,
        conn_id: i32,
        name: &str,
        schema: &mut Schema,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        let target = self.find_savepoint(conn_id, name, "ROLLBACK TO SAVEPOINT")?;
        self.check_lockstep(conn_id, storage)?;
        self.drop_frames_above(conn_id, target, storage)?;
        let frame_swaps_at_open = self.frames[target].sequence_swaps_at_open;
        if self.holds_writer {
            storage.rollback_to_savepoint(conn_id)?;
            // The frame keeps its snapshot for the next `ROLLBACK TO`: `restore_from`
            // takes the stamps and the counters it does not restore from the live
            // catalog, so the frame needs no re-baselining.
            let snapshot = self.frame_catalog(&self.frames[target], schema);
            let moved = enums_moved_by_restore(schema, &snapshot);
            schema.restore_from(snapshot, &self.sequence_swaps[frame_swaps_at_open..]);
            resync_index_specs(schema, storage, conn_id, &moved)?;
        }
        // Every counter-cell swap recorded since this frame opened has just been
        // undone, so the log truncates back to its baseline — which is what clears
        // those swaps for the *enclosing* frames too, and leaves any swap made before
        // this frame opened standing for them.
        self.sequence_swaps.truncate(frame_swaps_at_open);
        // A successful `ROLLBACK TO` clears the aborted state (measured on 18.6).
        self.status = TransactionStatus::InTransaction;
        Ok(())
    }

    /// Drop the savepoint named `name` and every savepoint established after it,
    /// **keeping** everything they wrote — the sub-work merges into the enclosing
    /// level.
    ///
    /// # Errors
    ///
    /// [`Error::NoActiveSqlTransaction`] (`25P01`) with no block open,
    /// [`Error::SavepointNotFound`] (`3B001`) when the block holds no live savepoint
    /// of that name, or the backend's error.
    pub fn release_savepoint(
        &mut self,
        conn_id: i32,
        name: &str,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        let target = self.find_savepoint(conn_id, name, "RELEASE SAVEPOINT")?;
        self.check_lockstep(conn_id, storage)?;
        self.drop_frames_above(conn_id, target, storage)?;
        storage.release_savepoint(conn_id)?;
        self.frames.pop();
        Ok(())
    }

    /// The index of the innermost live frame named `name`.
    ///
    /// **Innermost, so a repeated name stacks rather than colliding**: `SAVEPOINT s;
    /// … ; SAVEPOINT s` leaves two frames, `ROLLBACK TO s` takes the second, and
    /// after `RELEASE s` drops it the first answers the next reference — measured on
    /// 18.6. `frames[0]` carries no name, so no lookup can unwind the whole block.
    fn find_savepoint(&self, conn_id: i32, name: &str, stmt: &'static str) -> Result<usize, Error> {
        if !self.owns(conn_id) {
            return Err(Error::NoActiveSqlTransaction { stmt });
        }
        self.frames
            .iter()
            .rposition(|f| f.name.as_deref() == Some(name))
            .ok_or_else(|| Error::SavepointNotFound(name.to_string()))
    }

    /// Drop every frame above `target`, keeping what they wrote.
    ///
    /// Both `ROLLBACK TO` and `RELEASE` destroy the savepoints established after
    /// their target (measured on 18.6: `SAVEPOINT a; SAVEPOINT b; ROLLBACK TO a`
    /// leaves `b` gone, `3B001`). Releasing rather than rolling each one back is
    /// deliberate and not a shortcut: `ROLLBACK TO` restores the target's own
    /// snapshot immediately afterwards, so restoring the intermediate ones first
    /// would be work thrown away.
    ///
    /// # Errors
    ///
    /// The backend's error if a level cannot be dropped.
    fn drop_frames_above(
        &mut self,
        conn_id: i32,
        target: usize,
        storage: &mut StorageBackend,
    ) -> Result<(), Error> {
        while self.frames.len() > target + 1 {
            storage.release_savepoint(conn_id)?;
            self.frames.pop();
        }
        Ok(())
    }

    /// Check that the backend's savepoint stack still stands one level per named
    /// frame here, before an operation that indexes into it.
    ///
    /// The backend's stack is anonymous and top-addressed **by design** — the names
    /// live here and nowhere else (see `pglet_core::Storage::savepoint`) — so this
    /// session picking frame *k* and the backend restoring its own top are the same
    /// operation only while the two stacks are the same height. `frames[0]` is the
    /// block itself, staged through `begin`/`commit`/`rollback` rather than the
    /// savepoint stack, so the counts differ by exactly one.
    ///
    /// A `DO` block pushes and pops the backend's stack inside a single
    /// statement; that is balanced by the time any of these run, which is what makes
    /// the check an equality rather than a bound.
    ///
    /// # Errors
    ///
    /// A loud [`Error::Execution`] on a mismatch, rather than restoring whichever
    /// level happens to be on top.
    fn check_lockstep(&self, conn_id: i32, storage: &StorageBackend) -> Result<(), Error> {
        let expected = self.frames.len().saturating_sub(1);
        let actual = storage.savepoint_depth(conn_id);
        if expected == actual {
            return Ok(());
        }
        Err(Error::Execution(format!(
            "savepoint stacks out of step: {expected} session frames vs {actual} storage levels (connection {conn_id})"
        )))
    }

    /// Return to `Idle` with no frames — the shared tail of `commit` and `rollback`.
    ///
    /// The backend's savepoint levels go here too, and **only** here: the two ends of a
    /// block are the one moment they are all gone, and the storage `commit`/`rollback`
    /// pair that looks like the same moment is also the per-statement auto-commit
    /// bracket (see [`StorageBackend::discard_savepoints`]).
    fn end(&mut self, conn_id: i32, storage: &mut StorageBackend) {
        storage.discard_savepoints(conn_id);
        self.status = TransactionStatus::Idle;
        self.frames.clear();
        self.holds_writer = false;
        self.sequence_swaps.clear();
        self.replaced_cells.clear();
        self.durable_catalog_at_begin = None;
        self.snapshot_stamp = None;
        self.catalog_lent = false;
    }

    /// Record that the host holds the catalog mutably while a block is open
    /// (`crate::Database::schema_mut`): a change made through the borrow moves no
    /// generation, and a replacement can set it to any value, so from here on every
    /// rollback of the block moves the generation ([`Self::frame_catalog`]). A no-op with
    /// no block open — the next `BEGIN`'s frame is taken after the change.
    pub const fn lend_catalog(&mut self) {
        if !matches!(self.status, TransactionStatus::Idle) {
            self.catalog_lent = true;
        }
    }

    /// The catalog `frame` holds, to be restored over `live` ([`Schema::restore_from`]).
    /// Its stamp is made unequal to the live one where the block lent the catalog
    /// ([`Self::lend_catalog`]), so the restore moves the generation past it rather than
    /// keep a stamp under which a plan of the host's catalog may be held.
    fn frame_catalog(&self, frame: &Frame, live: &Schema) -> Schema {
        let mut catalog = Schema::clone(&frame.schema);
        if self.catalog_lent {
            catalog.generation = live.generation.wrapping_add(1);
        }
        catalog
    }
}

/// The catalog a store's durable state holds, decoded at most once however many catalogs
/// it is compared with.
struct DurableCatalog<'a> {
    bytes: &'a [u8],
    /// `None` until a comparison needs it.
    decoded: Option<Result<Schema, Error>>,
}

impl<'a> DurableCatalog<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self {
            bytes,
            decoded: None,
        }
    }

    /// Whether it differs from `catalog`, whose save encoding is `encoded`, in anything
    /// but a counter advanced in place.
    ///
    /// Compared whole — every table, index, enum and sequence fact, in the encoding a save
    /// writes — except the counter of a sequence outside `replaced_cells`: `nextval` and
    /// `setval` advance a counter in place, and no rollback takes that back
    /// (`Schema::restore_from`), so a durable counter behind the live one is no change of
    /// a block's. A cell in `replaced_cells` has its counter compared: one a statement of
    /// the block replaced — `ALTER SEQUENCE … RESTART`, `TRUNCATE … RESTART IDENTITY`,
    /// `DROP SEQUENCE`, a `DROP TABLE`, `DROP COLUMN` or `DROP SCHEMA public` that drops a
    /// sequence, top-level or inside a `DO` statement (as [`TransactionSession::record_statement`]
    /// records them) — whose counter a rollback puts back.
    ///
    /// Content, not a record of statements: whatever changed the catalog — a statement, or
    /// a host writing it directly through `Database::schema_mut` — a save that wrote the
    /// change shows here. `true` where the durable catalog does not decode, the cautious
    /// answer.
    fn differs(&mut self, catalog: &Schema, encoded: &[u8], replaced_cells: &[String]) -> bool {
        if self.bytes == encoded {
            return false;
        }
        let bytes = self.bytes;
        // A store no save has written a catalog to reopens on an empty one.
        let decoded = self.decoded.get_or_insert_with(|| {
            if bytes.is_empty() {
                Ok(Schema::default())
            } else {
                deserialize_schema(bytes)
            }
        });
        let Ok(durable) = decoded else {
            return true;
        };
        let advanced_in_place = |(name, sequence): (&String, &Sequence)| {
            !replaced_cells.contains(name)
                && catalog
                    .sequences
                    .get(name)
                    .is_some_and(|live| live.last_value != sequence.last_value)
        };
        // The bytes differ; with no counter to set aside, in something else. That reads
        // the durable bytes as the encoding of what they decode to, which they are: a
        // comparison runs only past a save since the block took the writer, and a save
        // wrote them from a catalog (and were they not, the answer would be the cautious
        // one).
        if !durable.sequences.iter().any(advanced_in_place) {
            return true;
        }
        let mut normalized = durable.clone();
        for (name, sequence) in &mut normalized.sequences {
            if !replaced_cells.contains(name)
                && let Some(live) = catalog.sequences.get(name)
            {
                sequence.last_value = live.last_value;
            }
        }
        serialize_schema(&normalized).map_or(true, |normalized| normalized != encoded)
    }
}

/// The answer for a refusal after which the statement may be partly durable: a
/// definite `Error::DurableWriteFailed` becomes the unknown-outcome
/// `Error::DurableWriteUnconfirmed`, same SQLSTATE; any other error is kept.
fn outcome_unknown(e: Error) -> Error {
    match e {
        Error::DurableWriteFailed { reason, disk_full } => {
            Error::DurableWriteUnconfirmed { reason, disk_full }
        }
        other => other,
    }
}

/// Which kind of block-ending statement `sql` is, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExitStatement {
    /// `COMMIT` / `END`, with the optional `TRANSACTION` / `WORK` noise word. In an
    /// aborted block Postgres converts these to a rollback and reports tag
    /// `ROLLBACK`.
    ImplicitRollback,
    /// A statement whose first word is `ROLLBACK`. An aborted block admits it, and what
    /// it then does to the block is its own: a whole-block `ROLLBACK` ends it, a
    /// `ROLLBACK TO SAVEPOINT` clears the abort and leaves it open. A spelling the
    /// parser does not model (`ROLLBACK AND CHAIN`) raises `42601` from the parser
    /// rather than `25P02` from this gate.
    Rollback,
}

/// Whether `sql` ends the transaction block — the statements an aborted block still
/// admits instead of answering `25P02`.
///
/// **The classifier reads the lexer, and that is the whole design.** Postgres decides
/// this on the parse tree (`IsTransactionExitStmt` takes a `Node *`), so the scanner
/// has already stripped comments before anything classifies. pglet's previous
/// classifier was a `starts_with` over the trimmed, ASCII-uppercased text, and every
/// such site has the same false-negative shape: after `BEGIN; SELECT * FROM
/// no_such_table;`, all four of `/* c */ COMMIT`, `/* c */ ROLLBACK`, `-- c⏎COMMIT`
/// and `-- c⏎ROLLBACK` answered `25P02` instead of ending the block — **and so did
/// every statement after them, `BEGIN` included**, until a bare or trailing-comment
/// exit arrived. 18.6 answers all eight arrangements `ROLLBACK` and returns to `Idle`.
/// Widening the string match is not the fix: a classifier taught to skip comments is a
/// second tokenizer, and pglet's lexer already nests block comments the way 18.6 does
/// (`/* a /* b */ c */ ROLLBACK` routes here for free).
///
/// Reading the lexer is *not* the same as reading the parser, and the difference is
/// deliberate: this gate runs **ahead** of parse analysis, exactly where Postgres's
/// does, so a statement whose analysis would fail is still classified by its leading
/// words. The tag-dispatch rule that decides what an *executed* statement did to the
/// block is the other half and lives in [`TransactionSession::apply_command`].
///
/// # What this admits, against `IsTransactionExitStmt`
///
/// Postgres admits `COMMIT`/`END`, `ROLLBACK`/`ABORT`, `ROLLBACK TO SAVEPOINT` and
/// `PREPARE TRANSACTION` (`tcop/postgres.c`). This admits `COMMIT`/`END` (with the
/// `TRANSACTION`/`WORK` noise word, and **excluding** an `AND [NO] CHAIN` tail) and
/// anything whose first token is the `ROLLBACK` keyword. It does **not** admit `ABORT`
/// or `PREPARE TRANSACTION`: pglet's parser models neither, so admitting them would
/// only move their `42601` from this gate to the parser. Pinned as such in
/// `exit_answers_that_diverge_from_the_reference`.
///
/// **This classifier and pglet's parser admit different sets, and the difference is
/// observable**, because a statement this admits is then handed to the parser while an
/// implicit rollback is converted without parsing anything. Measured at `Parse` with no
/// transaction open — the parser's own question, with no gate in the way — pglet
/// against 18.6: `COMMIT` / `COMMIT WORK` / `COMMIT TRANSACTION` and `ROLLBACK` /
/// `ROLLBACK WORK` / `ROLLBACK TRANSACTION` are `ParseComplete` on both, while `END` /
/// `END WORK` / `END TRANSACTION`, `ABORT` / `ABORT WORK`, `ROLLBACK AND CHAIN`,
/// `COMMIT AND CHAIN` and `PREPARE TRANSACTION 'g1'` are `42601` here and
/// `ParseComplete` there (`ROLLBACK TO SAVEPOINT sp` was in that list and now parses). So the consequences split by route:
///
/// - **`END`** over a simple query is `CommandComplete(ROLLBACK)` and the block ends —
///   byte-identical to 18.6, because the implicit-rollback branch converts it and the
///   parser is never reached. Over the **extended** protocol the same text is `42601`
///   at `Parse`, where 18.6 answers `ParseComplete`: the `Parse` arm returns on a
///   `42601` before it consults this exemption.
/// - **`ROLLBACK TO SAVEPOINT sp`** is admitted here and then *runs*: the parser models it
///   now, so both spellings of 18.6's answer are reproduced — `3B001 savepoint "sp" does
///   not exist` with the block left failed when no savepoint was taken, and
///   `CommandComplete(ROLLBACK)` with the block back at `[T]` when one was. It is the only
///   statement an aborted block admits that can *clear* the abort, which is what the
///   admission is for.
/// - **`ROLLBACK AND CHAIN`** is admitted here and `42601` from the parser, and needs
///   no added precondition: over a simple query after a failure, 18.6 answers
///   `CommandComplete(ROLLBACK)` and leaves `[T]`, a fresh healthy block, where pglet
///   leaves `[E]`. `ROLLBACK WORK AND CHAIN` and `ROLLBACK TRANSACTION AND CHAIN`
///   chain there too, while `ROLLBACK AND NO CHAIN` ends the block (`[I]`); all four
///   are `42601` here. Admitting them **is conformance, not the defect** —
///   `ROLLBACK opt_transaction opt_transaction_chain` is a `TRANS_STMT_ROLLBACK`
///   (`gram.y`) and `IsTransactionExitStmt` admits it — so the divergence belongs to
///   the parser, and no assertion on this classifier can freeze it.
/// - The **`COMMIT`** spellings of the same modifier take the other route: the `AND`
///   tail is rejected here, so `COMMIT AND CHAIN` and `COMMIT AND NO CHAIN` are
///   `25P02` rather than `42601`, against `[T]` and `[I]` respectively on 18.6.
///
/// A statement the lexer cannot tokenize is not an exit statement — the leading tokens
/// it does read decide, and a lex error before the first keyword yields `None`. That
/// matches the previous behaviour and costs nothing: such a statement is refused by
/// this gate and would be `42601` from the parser anyway.
#[must_use]
pub fn classify_transaction_exit(sql: &str) -> Option<ExitStatement> {
    let head = lead_tokens(sql, 4);
    match head.first()?.kind {
        TokenKind::Keyword(Keyword::Commit | Keyword::End) => {
            // Skip the optional `TRANSACTION` / `WORK` noise word, then reject an
            // `AND [NO] CHAIN` tail. pglet models no transaction chaining (the parser
            // rejects `AND CHAIN`), so those spellings stay a refusal rather than
            // half-honoring the modifier by silently dropping the chain.
            let mut rest = &head[1..];
            if matches!(
                rest.first().map(|t| &t.kind),
                Some(TokenKind::Keyword(Keyword::Transaction))
            ) || matches!(rest.first(), Some(t) if is_word(t, sql, "work"))
            {
                rest = &rest[1..];
            }
            if matches!(
                rest.first().map(|t| &t.kind),
                Some(TokenKind::Keyword(Keyword::And))
            ) {
                return None;
            }
            Some(ExitStatement::ImplicitRollback)
        }
        TokenKind::Keyword(Keyword::Rollback) => Some(ExitStatement::Rollback),
        _ => None,
    }
}

/// Whether `sql`'s first statement ends the transaction block, in either form — the
/// boolean the protocol phases that only need "is this admitted" read.
#[must_use]
pub fn is_transaction_exit_stmt(sql: &str) -> bool {
    classify_transaction_exit(sql).is_some()
}

#[cfg(test)]
mod classifier_tests {
    //! Which statements an aborted block admits. The set is shared by every phase
    //! that gates the state, so these answers are the contract for all of them.

    use super::{ExitStatement, classify_transaction_exit, is_transaction_exit_stmt};

    #[test]
    fn plain_commit_and_end_are_an_implicit_rollback() {
        for sql in [
            "COMMIT",
            "COMMIT;",
            "commit",
            "  COMMIT  ",
            "COMMIT WORK",
            "COMMIT TRANSACTION",
            "COMMIT TRANSACTION;",
            "END",
            "END;",
            "end",
            "END WORK",
            "END TRANSACTION",
        ] {
            assert_eq!(
                classify_transaction_exit(sql),
                Some(ExitStatement::ImplicitRollback),
                "{sql:?}"
            );
        }
    }

    #[test]
    fn identifiers_and_other_statements_are_not_exits() {
        for sql in [
            "COMMITTED",
            "COMMITS",
            "ENDS",
            "ENDING",
            "COMMITMENT",
            "ROLLBACKX",
            "SELECT 1",
            "BEGIN",
            "SET x = 'y'",
            "SAVEPOINT sp",
        ] {
            assert_eq!(classify_transaction_exit(sql), None, "{sql:?}");
        }
    }

    #[test]
    fn a_leading_comment_no_longer_hides_the_exit() {
        // The wedge this classifier was rewritten to close: under the `starts_with`
        // text match all of these answered `25P02` **and so did every statement after
        // them**, `BEGIN` included, until a bare exit arrived. 18.6 ends the block on
        // all of them. The nested-block spelling is in the list because it is what
        // rules out "strip a leading comment" as the fix — matching it needs the
        // lexer's own nesting.
        for sql in [
            "/* c */ COMMIT",
            "-- c\nCOMMIT",
            "/* a /* b */ c */ COMMIT",
            "/* c */ /* d */ END WORK",
        ] {
            assert_eq!(
                classify_transaction_exit(sql),
                Some(ExitStatement::ImplicitRollback),
                "{sql:?}"
            );
        }
        for sql in [
            "/* c */ ROLLBACK",
            "-- c\nROLLBACK",
            "/* a /* b */ c */ ROLLBACK",
        ] {
            assert_eq!(
                classify_transaction_exit(sql),
                Some(ExitStatement::Rollback),
                "{sql:?}"
            );
        }
        // A carriage return ends a `--` comment, as it does on 18.6: the exit after it
        // is read, and a modifier after it is not swallowed into the comment.
        assert_eq!(
            classify_transaction_exit("-- c\rCOMMIT"),
            Some(ExitStatement::ImplicitRollback)
        );
        assert_eq!(
            classify_transaction_exit("-- c\rROLLBACK"),
            Some(ExitStatement::Rollback)
        );
        assert_eq!(
            classify_transaction_exit("COMMIT --x\rAND CHAIN"),
            classify_transaction_exit("COMMIT AND CHAIN"),
            "a modifier after a carriage return is part of the statement"
        );
        assert_ne!(
            classify_transaction_exit("COMMIT --x\rAND CHAIN"),
            classify_transaction_exit("COMMIT"),
        );
        // The controls: the bare and trailing-comment spellings agreed on both
        // engines throughout, and must keep agreeing.
        assert_eq!(
            classify_transaction_exit("COMMIT /* t */"),
            Some(ExitStatement::ImplicitRollback)
        );
        assert_eq!(
            classify_transaction_exit("ROLLBACK /* t */"),
            Some(ExitStatement::Rollback)
        );
    }

    #[test]
    fn a_non_ascii_keyword_spelling_is_not_an_exit() {
        // U+0131 LATIN SMALL LETTER DOTLESS I uppercases to ASCII `I`, so a Unicode
        // fold read `COMMıT` as a statement that exits the block, where 18.6 answers
        // `42601 syntax error at or near "COMMıT"`. The lexer reads it as an ordinary
        // identifier, which is the same answer reached without any fold of our own.
        for sql in ["COMMıT", "ROLLBAſK"] {
            assert!(!is_transaction_exit_stmt(sql), "{sql:?}");
        }
        assert!(is_transaction_exit_stmt("commit"));
        assert!(is_transaction_exit_stmt(" RollBack ;"));
    }

    #[test]
    fn a_chain_modifier_splits_the_two_keywords() {
        // `COMMIT AND [NO] CHAIN` is excluded (pglet's parser rejects the modifier, so
        // admitting it would only move the `42601`), while `ROLLBACK AND CHAIN` is
        // admitted — not by accident of a prefix match but as conformance:
        // `ROLLBACK opt_transaction opt_transaction_chain` is a `TRANS_STMT_ROLLBACK`
        // (`gram.y`), which `IsTransactionExitStmt` admits. What diverges on that
        // statement is pglet's parser, not this answer.
        for sql in [
            "COMMIT AND CHAIN",
            "COMMIT AND NO CHAIN",
            "COMMIT WORK AND CHAIN",
            "COMMIT TRANSACTION AND CHAIN",
            "END AND CHAIN",
            "/* c */ COMMIT AND CHAIN",
        ] {
            assert_eq!(classify_transaction_exit(sql), None, "{sql:?}");
        }
        for sql in [
            "ROLLBACK AND CHAIN",
            "ROLLBACK TO SAVEPOINT sp",
            "ROLLBACK WORK AND CHAIN",
        ] {
            assert_eq!(
                classify_transaction_exit(sql),
                Some(ExitStatement::Rollback),
                "{sql:?}"
            );
        }
    }

    #[test]
    fn exit_answers_that_diverge_from_the_reference() {
        // These pin **pglet's answer, which is not the requirement** — kept as a
        // change-detector and split out so nothing reads them as conformance.
        // `ABORT opt_transaction opt_transaction_chain` is a `TRANS_STMT_ROLLBACK`
        // (`gram.y`) and `PREPARE TRANSACTION Sconst` a `TRANS_STMT_PREPARE`, both of
        // which `IsTransactionExitStmt` admits — so 18.6 answers **true** for every
        // spelling here and this function answers false, which is why an aborted block
        // gives each of them `25P02`. Measured on 18.6 after `BEGIN; SELECT * FROM
        // <absent>;`: `ABORT` answers `CommandComplete(ROLLBACK)` and ends the block.
        //
        // It is the member that makes this a gate rather than a note: the obvious port
        // of `IsTransactionExitStmt` adds arms here, which would let the statements
        // past this gate and into the parser's `42601` — and, without this line, redden
        // nothing. Reading these as the requirement and "fixing" the classifier alone
        // closes no divergence; the repair starts at the parser, and this test then
        // goes red on purpose.
        for sql in ["ABORT", "ABORT WORK", "PREPARE TRANSACTION 'g1'"] {
            assert!(
                !is_transaction_exit_stmt(sql),
                "pinned divergence — 18.6 answers true here: {sql:?}"
            );
        }
    }

    #[test]
    fn an_unlexable_statement_is_not_an_exit() {
        // A lex error before the first keyword yields no tokens; a well-formed leading
        // keyword still classifies even when the rest cannot be lexed, which is the
        // point of reading only the head.
        assert!(!is_transaction_exit_stmt("'unterminated"));
        assert!(!is_transaction_exit_stmt("/* unterminated"));
        assert!(is_transaction_exit_stmt("ROLLBACK 'unterminated"));
    }
}
