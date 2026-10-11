//! In-process embed API for the pglet database engine.
//!
//! `Database` is the single-writer, single-threaded engine: it holds the
//! `Schema` catalog, a `StorageBackend` and a [`TransactionSession`], and runs the
//! parse → plan → execute pipeline directly. There are no locks, no async, and no
//! sockets here — a server (or any multi-connection host) layers its own
//! concurrency, per-socket session state, and wire encoding on top, delegating
//! engine assembly, statement execution **and the transaction block** to this
//! crate. The block is a value type rather than a `Database` method precisely so
//! the native server, which never holds a `Database`, drives the same one; see
//! [`TransactionSession`].
//!
//! # Constructors
//!
//! - `Database::open` — native, file-backed (double-meta recovery + schema restore).
//! - `Database::with_btree` — in-memory CoW B+tree (no persistence).
//! - `Database::with_btree_for_one_connection` — the same, for a host that runs every
//!   statement on one connection: another connection's transaction is refused, and a
//!   commit encodes no catalog (the wasm in-memory database).
//! - `Database::in_memory` — plain in-memory map (no MVCC/persistence); native
//!   only, behind the default `memory_storage` feature the wasm build leaves off.
//! - `Database::with_storage` — inject an already-constructed `StorageBackend`
//!   and `Schema`. This is the seam for embedding a custom `PageStore`: wrap it
//!   in a `BtreeStorageWrapper`, hand `StorageBackend::Btree(..)` here, and you
//!   have a `Database` without going through any server.
//!
//! `Database::fork` derives a fresh, isolated database from one over an in-memory
//! B+tree — a copy-on-write branch of its committed state, the per-test-fixture
//! primitive (seed a base once, fork a cheap independent copy per test).

// Without `memory_storage` (the wasm build) `StorageBackend` has one variant, and these
// lints read every dispatch written for two as redundant: an `if let` over the B+tree
// arm is irrefutable, a `_` arm matches one variant, and the helpers that touch only the
// `Memory` arm have nothing left to be non-`const` or `&mut` about. Rewriting them per
// configuration would fork the dispatch; the two-variant build keeps the lints.
#![cfg_attr(
    not(feature = "memory_storage"),
    allow(
        irrefutable_let_patterns,
        clippy::match_wildcard_for_single_variants,
        clippy::missing_const_for_fn,
        clippy::needless_pass_by_ref_mut,
        clippy::clone_on_copy
    )
)]

mod backend;
mod dump;
mod plan_cache;
mod transaction;

// Native-only imports for the file-backed `open`/`open_existing` constructors
// (std::fs + `FileBlockIo`); gated out of the wasm build, whose `opfs` feature
// assembles its store over OPFS through `with_storage`.
#[cfg(feature = "file_store")]
use std::io;
#[cfg(feature = "file_store")]
use std::path::Path;

use pglet_core::Storage;
use pglet_sql::catalog::Schema;
use pglet_sql::depth::CatalogDepth;
use pglet_sql::executor::index_spec_from_catalog;
use pglet_sql::executor::{
    CopyParams, EvalEnv, QueryResult, SequenceSession, TransactionCommand, copy_column_count,
    execute_copy_from, execute_copy_to, execute_with_storage,
};
use pglet_sql::planner::{CopyDirection, Plan, bound_param_enum_legality, infer_param_targets};
use pglet_sql::types::{TypeOid, Value};
#[cfg(feature = "memory_storage")]
use pglet_storage::MemoryStorage;
use pglet_storage::btree::BtreeStorageWrapper;
use plan_cache::{PlanCache, Prepared};

#[cfg(feature = "file_store")]
use pglet_storage::btree::{CowBtree, DoubleMetaStore, FileBlockIo};

pub use backend::{SnapshotAdapter, StorageAdapter, StorageBackend};
pub use transaction::{
    AbortedBlockAction, ExitStatement, TransactionSession, autocommit_is_discardable,
    autocommit_publish_bytes, classify_transaction_exit, is_transaction_exit_stmt,
    plan_failure_is_the_writers,
};

/// A [`Database`]'s session settings, one set per logical connection — as the native
/// server holds one per socket — so a `SET` a connection makes, its `SET TRANSACTION`
/// and a rollback of its block move its own settings and no other connection's.
///
/// A connection's settings are held only while they differ from what a connection starts
/// with ([`Self::template`]): this facade has no event that ends a logical connection, so
/// an entry equal to the template — one nothing moved, or one moved back — is dropped
/// the next time a connection is given one, and a host minting connection ids holds no
/// entry per id it used. A list scanned by id rather than a map: the wasm build holds one
/// connection, and a map's code costs it more than a scan costs a host with a handful.
#[derive(Debug, Default)]
struct ConnectionSettings {
    /// What a connection's settings start as: the defaults, and a host's own
    /// `lock_timeout` ([`Database::set_default_lock_timeout`]).
    template: pglet_sql::catalog::SessionSettings,
    by_connection: Vec<(i32, pglet_sql::catalog::SessionSettings)>,
}

impl ConnectionSettings {
    /// `conn_id`'s settings — the template's until it has moved one.
    fn of(&self, conn_id: i32) -> &pglet_sql::catalog::SessionSettings {
        self.by_connection
            .iter()
            .find(|(id, _)| *id == conn_id)
            .map_or(&self.template, |(_, settings)| settings)
    }

    /// `conn_id`'s settings, to move. Giving a connection an entry first drops every
    /// entry that equals the template.
    fn of_mut(&mut self, conn_id: i32) -> &mut pglet_sql::catalog::SessionSettings {
        let found = self.by_connection.iter().position(|(id, _)| *id == conn_id);
        let at = found.unwrap_or_else(|| {
            let template = &self.template;
            self.by_connection
                .retain(|(_, settings)| settings != template);
            self.by_connection.push((conn_id, template.clone()));
            self.by_connection.len() - 1
        });
        &mut self.by_connection[at].1
    }

    /// A host's own `lock_timeout` for every connection, and its `RESET` target.
    fn set_default_lock_timeout(&mut self, ms: u64) {
        self.template.set_default_lock_timeout(ms);
        for (_, settings) in &mut self.by_connection {
            settings.set_default_lock_timeout(ms);
        }
    }

    /// The settings a fork's connections start with: the template, a host's own
    /// `lock_timeout` included, and no connection's moved settings.
    fn fork(&self) -> Self {
        Self {
            template: self.template.clone(),
            by_connection: Vec::new(),
        }
    }
}

/// Filename of the single-file double-meta store inside a `--data-dir`.
#[cfg(feature = "file_store")]
const DATA_FILENAME: &str = "data.pglet";

// Re-export the engine surface a host needs so it can depend on `pglet` alone.
pub use pglet_core::TransactionStatus;
pub use pglet_sql::Error;
pub use pglet_sql::executor::QueryResult as ExecuteResult;

/// Set the calling thread's statement-depth limit — how many levels a statement's tree
/// may nest before it is refused `54001` — returning the previous one.
///
/// Every thread starts with a limit sized for the stack it can be assumed to have: 2 MiB
/// where the engine knows the thread's stack — declared ([`set_thread_stack`]), or the
/// main thread's on Linux — (`pglet_sql::depth::DEFAULT_MAX_DEPTH`), and on every other
/// thread the limit sized with the budget the engine measures there, below the facade
/// entry (`pglet_sql::depth::FALLBACK_MAX_DEPTH`, with `DEFAULT_STACK_BUDGET`). Raise it
/// on a thread
/// with more stack — declared with [`set_thread_stack`] unless the OS can say it is
/// there — to answer deeper statements, and to open a store written where the limit was
/// larger ([`StoreTooDeep`]): the native server's connection threads admit
/// `pglet_sql::depth::SERVER_MAX_DEPTH` levels on `pglet_sql::depth::SERVER_THREAD_STACK`
/// bytes. Lower it on a thread smaller than those defaults assume.
///
/// **The stack must hold the limit.** The engine keeps free below its byte check's own
/// limit a reserve that grows with this one (`pglet_sql::depth::stack_reserve`), for the
/// walks that cannot check the stack; raising the limit on a thread without that room
/// is how a statement overflows it and aborts the process. The engine knows only a
/// declared stack and a Linux main thread's; on any other thread it measures a budget
/// below the point a statement enters the facade (`pglet_sql::depth::DEFAULT_STACK_BUDGET`),
/// so declare a larger thread's size with [`set_thread_stack`] — or bound it per database
/// with [`Database::set_stack_budget`] — before raising its limit.
///
/// The limit is the thread's, not a database's: it holds for every database used on
/// this thread, and a statement run on another thread takes that thread's.
pub fn set_thread_max_depth(depth: usize) -> usize {
    pglet_sql::depth::set_max_depth(depth)
}

/// Declare the calling thread's stack: `bytes` in all, the calling frame near its top.
///
/// Call it first thing on a thread the host sized, before any statement runs there. The
/// engine then measures the thread's statements against this stack, and starts it at the
/// limit sized for 2 MiB (`pglet_sql::depth::DEFAULT_MAX_DEPTH`). Without it only a
/// Linux main thread's stack is known: `/proc/self/maps` cannot say which thread's a
/// mapping's low end is (two adjacent stacks share one mapping), and there is no
/// `/proc` off Linux. The native build only; wasm has no byte check.
///
/// On Linux a declaration larger than the mapping the thread runs in is held to the
/// mapping. What the engine cannot check stays the caller's contract: that `bytes` is
/// no more than the thread has, off Linux and on a thread whose mapping has merged with
/// a neighbour's below it (`pglet_sql::depth::set_thread_stack`).
#[cfg(not(target_arch = "wasm32"))]
pub fn set_thread_stack(bytes: usize) {
    pglet_sql::depth::set_thread_stack(bytes);
}

/// [`Database::open`]'s `54001`: the store holds a stored expression deeper than the
/// opening thread reads.
///
/// The expression is a `CHECK`, a column `DEFAULT`, an index expression or predicate, and
/// the refusal is raised before anything of the store is open.
///
/// A deeper store is one written where the limit was larger — typically by the native
/// server, whose connection threads admit `pglet_sql::depth::SERVER_MAX_DEPTH` levels.
/// Every stored expression is decoded under the opener's own bounds, so the opener
/// refuses rather than overflowing its stack. The ways to read such a store: open **and
/// use** it on a thread with a larger stack, declared ([`set_thread_stack`]) and its
/// limit raised ([`set_thread_max_depth`]) — a thread of
/// `pglet_sql::depth::SERVER_THREAD_STACK` bytes at `SERVER_MAX_DEPTH` opens what the
/// server wrote — or export it with `pglet_server dump --data-dir <dir>`, which reads it
/// on such a thread.
///
/// Opening is not the only step the bounds hold. A statement walks the whole catalog —
/// a `BEGIN` snapshots it, every commit encodes it — under the **calling** thread's
/// bounds, so the same database used on a smaller thread than the one that opened it,
/// or under a limit lowered or a budget narrowed ([`Database::set_stack_budget`])
/// since, is refused every statement `54001` at the facade's entry, before anything of
/// the statement runs.
///
/// It reaches a caller as the inner error of the `io::Error` [`Database::open`] returns.
#[derive(Debug)]
pub struct StoreTooDeep {
    max_depth: usize,
}

impl StoreTooDeep {
    /// The opening thread's limit. A stored expression exceeded it, or the stack the
    /// engine knows the thread has — the budget below the open, on a thread it does not
    /// know.
    #[must_use]
    pub const fn max_depth(&self) -> usize {
        self.max_depth
    }

    /// `54001`, `statement_too_complex` — the SQLSTATE of the refusal inside the open.
    #[must_use]
    pub const fn sqlstate(&self) -> &'static str {
        "54001"
    }
}

/// The in-statement refusal the open raised, as [`StoreTooDeep`]'s source.
static STORE_TOO_DEEP_SOURCE: Error = Error::StackDepthExceeded;

impl std::fmt::Display for StoreTooDeep {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "stack depth limit exceeded opening the store: it holds a stored expression deeper \
             than this thread reads — at most {} levels, within the stack the engine knows it \
             has. Open and use it on a thread with a larger stack, declared with \
             `pglet::set_thread_stack` and its limit raised with `pglet::set_thread_max_depth` \
             (a thread of {} MiB at {} levels opens what a `pglet_server` wrote) — a database \
             is refused every statement on a thread that could not walk the catalog — or \
             export it with `pglet_server dump --data-dir <dir>`",
            self.max_depth,
            pglet_sql::depth::SERVER_THREAD_STACK / (1024 * 1024),
            pglet_sql::depth::SERVER_MAX_DEPTH,
        )
    }
}

impl std::error::Error for StoreTooDeep {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        Some(&STORE_TOO_DEEP_SOURCE)
    }
}

/// The `io::Error` an open answers for `error`: [`StoreTooDeep`] for the depth refusal,
/// else `other` built from it.
#[cfg(feature = "file_store")]
fn open_error(error: Error, other: impl FnOnce(Error) -> io::Error) -> io::Error {
    match error {
        Error::StackDepthExceeded => io::Error::other(StoreTooDeep {
            max_depth: pglet_sql::depth::max_depth(),
        }),
        e => other(e),
    }
}

/// The two error domains [`Database::execute_bound`] threads.
///
/// A bound execute both runs the engine (parse / plan / execute, an [`Error`])
/// and calls back into a host to decode parameters (an arbitrary `E`). Keeping
/// the domains distinct lets a host surface each in its own form — e.g. the wasm
/// bindings pass a decode error (`JsValue`) straight through while mapping an
/// engine error to a JS error carrying its SQLSTATE — instead of flattening both
/// into one shape.
#[derive(Debug)]
pub enum BoundExecError<E> {
    /// A parse / plan / execute error from the engine.
    Engine(Error),
    /// The caller's parameter-binding callback failed.
    Bind(E),
}

impl<E> From<Error> for BoundExecError<E> {
    fn from(error: Error) -> Self {
        Self::Engine(error)
    }
}

/// The single-writer, single-threaded embedded database engine.
///
/// Holds the schema catalog and storage backend and runs SQL directly. Clone-free
/// and lock-free: callers needing concurrency wrap this externally.
#[derive(Debug)]
pub struct Database {
    schema: Schema,
    storage: StorageBackend,
    /// The sequence functions' session state (`currval`'s last-`nextval` cache). One
    /// per `Database`, shared by every logical connection
    /// ([`Database::execute_as`]), where the native server keeps one per connection (the
    /// repo's `EDGES.md` row 223). Kept outside `Schema` (which persists) because
    /// `currval` is a per-session, non-durable notion.
    sequence_session: SequenceSession,
    /// The runtime parameters whose value belongs to the session — `application_name`,
    /// `lock_timeout`, the isolation level — one set per logical connection.
    ///
    /// This host has no startup packet and no wire, so each starts at a fresh
    /// connection's default — `application_name` empty, `lock_timeout` the host's
    /// ([`Database::set_default_lock_timeout`], else
    /// [`pglet_sql::catalog::DEFAULT_LOCK_TIMEOUT_MS`]), the isolation level `read
    /// committed` — and never sends a `ParameterStatus`; it carries the value so `SHOW`
    /// answers the same thing here as it does over the socket.
    settings: ConnectionSettings,
    /// The transaction block: its status, its frame stack, and whether it holds the
    /// writer.
    /// Shared with the native server, which holds one per socket — see
    /// [`TransactionSession`] for why the state machine is a value type rather than
    /// a method on this struct.
    tx: TransactionSession,
    /// The stack a statement may use below the facade entry, where the caller bounded
    /// it ([`Self::set_stack_budget`]); `None` measures the thread's own stack.
    stack_budget: Option<usize>,
    /// The catalog's deepest stored expression, asked at every entry
    /// ([`Self::enter`]).
    catalog_depth: CatalogDepth,
    /// Whether the drop rolls back a block left open — cleared by
    /// [`Self::skip_drop_rollback`].
    drop_rolls_back: bool,
    /// The statements [`Self::execute_bound`] has parsed, by text.
    plan_cache: PlanCache,
}

impl Database {
    /// Open a file-backed database in the single-file double-meta layout,
    /// recovering the last consistent committed state and restoring the
    /// persisted schema.
    ///
    /// This is the native, persistent constructor. Data is written to a single
    /// file under `data_dir` and survives restarts. Recovery happens inside
    /// `DoubleMetaStore::open`, which selects the valid higher-version meta slot
    /// (falling back to the older slot if the newer one is torn). Crash safety
    /// is provided by the double-meta flip — there is no WAL.
    /// Unlike the server's `--data-dir` path, this does **not** take the
    /// data-dir process lock — single-process embed callers own that policy (the
    /// server adds the lock in its own layer).
    ///
    /// # Errors
    ///
    /// Returns an error if the data file can't be opened, the store has no valid
    /// meta slot (unrecoverable corruption), or the persisted schema can't be
    /// deserialized — and [`StoreTooDeep`] (`54001`) as the `io::Error`'s inner error
    /// when a stored expression is deeper than this thread's statement-depth limit,
    /// which names the ways to open it: a thread with more stack, declared
    /// ([`set_thread_stack`]) and its limit raised ([`set_thread_max_depth`]), or
    /// `pglet_server dump --data-dir <dir>`.
    #[cfg(feature = "file_store")]
    pub fn open(data_dir: &Path) -> io::Result<Self> {
        // Decoding the catalog walks every stored expression.
        let _stack = pglet_sql::depth::enter_stack(None);
        // Open or create the single-file double-meta store. Recovery is internal:
        // it picks the valid higher-version meta slot.
        std::fs::create_dir_all(data_dir)?;
        let file_io = FileBlockIo::open(&data_dir.join(DATA_FILENAME))?;
        let (page_store, metadata) = DoubleMetaStore::open(Box::new(file_io))
            .map_err(|e| io::Error::other(format!("open double-meta store: {e}")))?;

        // Build the CowBtree over the recovered root/version. The store has
        // already selected the latest consistent meta slot, so there is nothing
        // to replay.
        let btree = CowBtree::with_store(
            Box::new(page_store),
            metadata.root,
            metadata.version,
            100, // vacuum_threshold
        );

        // Recover the table directory, row-id counters, and catalog from the tree
        // (the PGM5 `0x02` system entries), not the meta slot. `next_table_id` from
        // the fixed slot bounds the SYS_TABLE scan.
        let (storage, schema_bytes) =
            BtreeStorageWrapper::recover(std::sync::Arc::new(btree), metadata.next_table_id)
                .map_err(|e| io::Error::other(format!("recover catalog from tree: {e}")))?;

        // Restore schema from the reassembled SYS_CATALOG bytes (or empty if none).
        let schema = if schema_bytes.is_empty() {
            Schema::new()
        } else {
            pglet_sql::catalog::deserialize_schema(&schema_bytes).map_err(|e| {
                open_error(e, |e| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("failed to deserialize schema: {e}"),
                    )
                })
            })?
        };

        // Re-register persisted indexes with the wrapper (see
        // `reattach_persisted_indexes`); the entries are in the tree, only the
        // in-memory def map needs restoring.
        let mut db = Self {
            schema,
            sequence_session: SequenceSession::new(),
            settings: ConnectionSettings::default(),
            tx: TransactionSession::new(),
            storage: StorageBackend::Btree(storage),
            stack_budget: None,
            catalog_depth: CatalogDepth::default(),
            drop_rolls_back: true,
            plan_cache: PlanCache::default(),
        };
        db.reattach_persisted_indexes().map_err(|e| {
            open_error(e, |e| io::Error::other(format!("re-register indexes: {e}")))
        })?;

        Ok(db)
    }

    /// Like [`Database::open`], but fails loud (`NotFound`) if no database exists
    /// under `data_dir`, rather than creating a fresh empty store.
    ///
    /// `open` deliberately creates the store on demand (the server's first-run
    /// path). A read-only consumer wants the opposite: the `dump` subcommand must
    /// reject a mistyped `--data-dir` instead of silently emitting an empty dump
    /// (and writing a stray empty store to the wrong path).
    ///
    /// # Errors
    ///
    /// Returns `NotFound` if `data_dir`'s data file is absent, plus every error
    /// [`Database::open`] can return.
    #[cfg(feature = "file_store")]
    pub fn open_existing(data_dir: &Path) -> io::Result<Self> {
        let data_file = data_dir.join(DATA_FILENAME);
        if !data_file.exists() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "no pglet database at {} ({DATA_FILENAME} not found)",
                    data_dir.display()
                ),
            ));
        }
        Self::open(data_dir)
    }

    /// Create a database over an in-memory CoW B+tree (no persistence).
    pub fn with_btree() -> Self {
        Self::over_btree(BtreeStorageWrapper::new())
    }

    /// A fresh database over `storage`, an in-memory B+tree.
    fn over_btree(storage: BtreeStorageWrapper) -> Self {
        Self {
            schema: Schema::new(),
            storage: StorageBackend::Btree(storage),
            sequence_session: SequenceSession::new(),
            settings: ConnectionSettings::default(),
            tx: TransactionSession::new(),
            stack_budget: None,
            catalog_depth: CatalogDepth::default(),
            drop_rolls_back: true,
            plan_cache: PlanCache::default(),
        }
    }

    /// [`Self::with_btree`] for a host that runs every statement on `conn_id`: any other
    /// connection's transaction — its block, and the one an autocommit write runs in — is
    /// refused ([`StorageBackend::begin`]), and a commit encodes no catalog, since nothing
    /// would read it: no reopen, and no other connection's block to compare the commit
    /// stamp its write moves ([`StorageBackend::persists_catalog`]). A [`Self::fork`] of
    /// it keeps both.
    #[must_use]
    pub fn with_btree_for_one_connection(conn_id: i32) -> Self {
        Self::over_btree(BtreeStorageWrapper::for_one_connection(conn_id))
    }

    /// Create a database over plain in-memory storage (no MVCC, no persistence).
    #[cfg(feature = "memory_storage")]
    pub fn in_memory() -> Self {
        Self {
            schema: Schema::new(),
            storage: StorageBackend::Memory(MemoryStorage::new()),
            sequence_session: SequenceSession::new(),
            settings: ConnectionSettings::default(),
            tx: TransactionSession::new(),
            stack_budget: None,
            catalog_depth: CatalogDepth::default(),
            drop_rolls_back: true,
            plan_cache: PlanCache::default(),
        }
    }

    /// Build a database over a caller-provided backend and schema.
    ///
    /// This is the generic storage-injection seam. To embed a custom `PageStore`,
    /// wrap it in a `BtreeStorageWrapper` (via `CowBtree::with_store` +
    /// `BtreeStorageWrapper::recover`) and pass `StorageBackend::Btree(..)`.
    /// The caller is responsible for any recovery the backend requires.
    // Not `const`: initializing the `SequenceSession`'s `HashMap` isn't const.
    pub fn with_storage(storage: StorageBackend, schema: Schema) -> Self {
        Self {
            // A caller's own catalog: measured at the first statement, since its
            // expressions came in by neither of the doors the process's bound reads
            // (`pglet_sql::depth::CatalogDepth`).
            schema,
            storage,
            sequence_session: SequenceSession::new(),
            settings: ConnectionSettings::default(),
            tx: TransactionSession::new(),
            stack_budget: None,
            catalog_depth: CatalogDepth::default(),
            drop_rolls_back: true,
            plan_cache: PlanCache::default(),
        }
    }

    /// Make the drop of this database write nothing: a block left open is not rolled back
    /// when it drops, and nothing is saved — the durable state stays as the last save left
    /// it, as after a kill.
    ///
    /// For a host that can no longer trust its own memory and must still let the database
    /// go: the wasm persistent binding, whose module refuses every call once one never
    /// returned, and whose close is the drop. Nothing else changes — the database serves
    /// on until it is dropped.
    pub const fn skip_drop_rollback(&mut self) {
        self.drop_rolls_back = false;
    }

    /// Decompose into the held `(schema, storage)`.
    ///
    /// A host that runs its own concurrency layer (separate locks over schema and
    /// storage, per-socket session state) builds the engine through `open`/`with_*` for
    /// the recovery + assembly logic, then takes ownership of the parts to place them
    /// behind its own synchronization.
    ///
    /// The [`TransactionSession`] is **dropped**, not returned: a host that splits the
    /// engine this way holds one session per connection rather than one per database,
    /// and builds them itself. A transaction open on this `Database` is therefore neither
    /// rolled back, as a drop rolls it back, nor ended: its staged writes stay open in the
    /// storage half. On a database rebuilt from the parts, that connection's next
    /// autocommit statement runs inside them — acknowledged, seen by no other connection,
    /// not durable — and its next block adopts them, so that block's `COMMIT`, an empty
    /// one included, commits the abandoned writes (the repo's `EDGES.md` row 216).
    /// Assembly-time use only, with no block open.
    pub fn into_parts(mut self) -> (Schema, StorageBackend) {
        // What is left behind is dropped with no block open and over an empty in-memory
        // store, so the drop does nothing.
        self.tx = TransactionSession::new();
        let schema = std::mem::take(&mut self.schema);
        let storage = std::mem::replace(
            &mut self.storage,
            StorageBackend::Btree(BtreeStorageWrapper::new()),
        );
        (schema, storage)
    }

    /// Fork this database into an independent copy of its current committed state.
    ///
    /// The per-test-fixture primitive: seed a base database once (schema + seed
    /// data), then `fork()` a fresh, isolated database per test instead of
    /// re-running the setup SQL. The fork shares the base's row data by `Arc`
    /// refcount until written — a copy-on-write branch of the [`with_btree`] CoW
    /// tree — so it is far cheaper than replaying the schema + seed, and orders of
    /// magnitude cheaper than spinning a fresh Postgres/pglite. Writes on either
    /// side copy only the pages they touch and are invisible to the other.
    ///
    /// Requires an in-memory CoW database, as [`with_btree`] builds —
    /// [`Self::with_btree_for_one_connection`]'s included, whose one connection the fork
    /// keeps. A file/OPFS-backed ([`open`]) or plain [`in_memory`] database can't be
    /// forked and fails loud.
    /// The fork reflects **committed** state and starts with no active transaction.
    /// Its connections' settings start as a fresh connection's here do: a host's
    /// [`set_default_lock_timeout`] carries, a connection's `SET` does not.
    ///
    /// # Why an open transaction is refused rather than ignored
    ///
    /// The two halves a fork copies come from different places: the storage tree is
    /// branched at its **committed** root, while the catalog is a clone of
    /// `self.schema`, which an open block has already mutated in place (no backend
    /// rolls the catalog back — see [`crate::transaction`]). Forking across that
    /// split hands the fork a catalog and a heap that disagree, and both directions
    /// are silent: a pending `DROP TABLE` hides a committed table **and its
    /// committed rows** from the fork (`42P01`), and a pending `ADD COLUMN` gives it
    /// a wider descriptor than its rows carry, which breaks every `SELECT *` at the
    /// wire. Since this is the seed-once/fork-per-test primitive, a caller that
    /// seeds inside an uncommitted block would get well-formed-looking empty
    /// fixtures. So the refusal is the right answer, and `25001` is the SQLSTATE
    /// Postgres uses for it.
    ///
    /// [`with_btree`]: Self::with_btree
    /// [`open`]: Self::open
    /// [`in_memory`]: Self::in_memory
    /// [`set_default_lock_timeout`]: Self::set_default_lock_timeout
    ///
    /// # Errors
    ///
    /// Returns an error if a transaction block is open on this database (`25001`),
    /// or if the backend isn't an in-memory CoW B+tree.
    pub fn fork(&self) -> Result<Self, Error> {
        let _stack = self.enter()?;
        if self.tx.status() != TransactionStatus::Idle {
            return Err(Error::ActiveSqlTransaction { op: "fork()" });
        }
        let storage = match &self.storage {
            StorageBackend::Btree(wrapper) => {
                wrapper.fork().map(StorageBackend::Btree).ok_or_else(|| {
                    Error::Execution(
                        "fork requires an in-memory B+tree database; a file/OPFS-backed store \
                         can't be forked"
                            .into(),
                    )
                })?
            }
            #[cfg(feature = "memory_storage")]
            StorageBackend::Memory(_) => {
                return Err(Error::Execution(
                    "fork requires a B+tree database (Database::with_btree); plain in-memory \
                     storage has no copy-on-write fork"
                        .into(),
                ));
            }
        };
        Ok(Self {
            schema: self.schema.clone(),
            storage,
            // A fork is a fresh session — the parent's `currval` state does not carry,
            // nor does any connection's `SET`; a host's own `lock_timeout` does.
            // Its transaction block cannot: the gate above refused one.
            sequence_session: SequenceSession::new(),
            settings: self.settings.fork(),
            tx: TransactionSession::new(),
            stack_budget: self.stack_budget,
            catalog_depth: CatalogDepth::default(),
            drop_rolls_back: true,
            // Not the parent's: from here each catalog moves its generation on its own,
            // so one number can name two catalogs.
            // TODO(perf): so sibling forks of one base each parse the statements they
            // share; sharing a cache across them needs a catalog stamp no two lineages
            // can reach.
            plan_cache: PlanCache::default(),
        })
    }

    /// Bound the stack a statement may use: `bytes` below the point it enters this
    /// facade, refusing `54001` past it. With `None` — the default — the bound is the
    /// calling thread's own stack less the engine's reserve where the engine knows it
    /// (declared with [`set_thread_stack`], or a Linux main thread's), else
    /// [`pglet_sql::depth::DEFAULT_STACK_BUDGET`]. A budget can narrow a known stack,
    /// never widen it. The wasm build has no byte check to bound, and ignores it.
    ///
    /// Empties the parsed statements [`Self::execute_bound`] holds, each parsed under the
    /// budget it replaces.
    pub fn set_stack_budget(&mut self, bytes: Option<usize>) {
        self.stack_budget = bytes;
        self.plan_cache.clear();
    }

    /// Borrow the schema catalog.
    pub const fn schema(&self) -> &Schema {
        &self.schema
    }

    /// Mutably borrow the schema catalog.
    ///
    /// The catalog's depth is measured again at the next statement, since a caller can
    /// change it here without the generation moving ([`CatalogDepth::forget`]) — or move
    /// the generation backwards, replacing the catalog — and for the same reason the
    /// parsed statements [`Self::execute_bound`] holds are dropped.
    ///
    /// **A catalog write, so with a block open it is the block's**: the block takes the
    /// writer here ([`TransactionSession::take_writer`]), before the caller's change, and
    /// its rollback takes the change back with its own and moves the generation, so no
    /// plan parsed against the host's catalog answers for the one restored. With no block
    /// open the change is the host's and stands, as any catalog change does, until the
    /// next save writes it.
    pub fn schema_mut(&mut self) -> &mut Schema {
        self.catalog_depth.forget();
        self.plan_cache.clear();
        if let Some(owner) = self.tx.owner() {
            self.tx
                .take_writer_for_host(owner, &self.schema, &mut self.storage);
        }
        self.tx.lend_catalog();
        &mut self.schema
    }

    /// Make `ms` this database's `lock_timeout` and the value a `RESET` restores (`0`:
    /// no bound), for every logical connection and every [`Self::fork`] of it — a host's
    /// setting, not a `SET`, so an open block's rollback does not take it back. This host
    /// never waits for the writer — another logical connection's write while a block
    /// holds it is refused `55P03` at once — so the value is what `SHOW lock_timeout`
    /// answers, the default being [`pglet_sql::catalog::DEFAULT_LOCK_TIMEOUT_MS`].
    pub fn set_default_lock_timeout(&mut self, ms: u64) {
        self.settings.set_default_lock_timeout(ms);
    }

    /// Take the writer for `conn_id`'s write — the embed's half of the rule that at most
    /// one block stages writes at a time. This host runs one statement at a time on one
    /// thread and so cannot make a write **wait**; it refuses instead, keyed as the
    /// server's wait is keyed: another connection's write only while the open block
    /// holds the writer (a block that has only read takes it at its first write), and a
    /// block's own write takes it — re-basing the block's frames on what other
    /// connections committed while it read ([`TransactionSession::take_writer`]).
    ///
    /// # Errors
    ///
    /// `55P03` ([`Error::WriterHeldElsewhere`]) when another connection's block holds
    /// the writer.
    fn claim_writer(&mut self, conn_id: i32) -> Result<(), Error> {
        match self.tx.owner() {
            Some(owner) if owner != conn_id => {
                if self.tx.holds_writer() {
                    Err(Error::WriterHeldElsewhere(format!(
                        "could not obtain the writer for connection {conn_id}: connection {owner}'s transaction block holds it, and an embedded database cannot wait"
                    )))
                } else {
                    Ok(())
                }
            }
            Some(_) => {
                let Self {
                    schema,
                    storage,
                    tx,
                    settings,
                    ..
                } = self;
                tx.take_writer(
                    conn_id,
                    settings.of(conn_id).block_uses_transaction_snapshot(),
                    schema,
                    storage,
                )
            }
            None => Ok(()),
        }
    }

    /// What `conn_id`'s failed plan answers: `error`, unless the plan failed against a
    /// catalog another connection's block has changed while holding the writer **and**
    /// that change is why — `replan`, the same statement planned against the catalog
    /// committed when the block took the writer, does not fail the same way
    /// ([`plan_failure_is_the_writers`]). Then the statement is one the native server
    /// makes wait for the block and plan again, so this host, which cannot wait, refuses
    /// it `55P03` as it refuses that block's writes. A failure of the statement's own — a
    /// syntax error, a column no catalog has, a cast no catalog decides — is its answer,
    /// where it is the first failure against both catalogs: a syntax error planning
    /// reaches only after a check the block's change decides (a set operation's column
    /// counts, for one) is refused `55P03`.
    fn plan_refusal<T>(
        &self,
        conn_id: i32,
        error: Error,
        replan: impl FnOnce(&Schema) -> Result<T, Error>,
    ) -> Error {
        let Some(committed) = self.tx.catalog_at_anothers_writer(conn_id, &self.schema) else {
            return error;
        };
        if !plan_failure_is_the_writers(&error, &replan(committed)) {
            return error;
        }
        let owner = self.tx.owner().unwrap_or_default();
        Error::WriterHeldElsewhere(format!(
            "could not plan the statement on connection {conn_id}: connection {owner}'s transaction block holds the writer and has changed the catalog, and an embedded database cannot wait"
        ))
    }

    /// Refuse `conn_id`'s `BEGIN`, `COMMIT` or `ROLLBACK` while another connection's
    /// block is open: this `Database` holds one transaction session, so the statement
    /// would open nothing or end the other connection's block. `55P03`, the answer the
    /// writer's refusal gives, since the block is what the connection would wait on.
    fn refuse_foreign_block_control(&self, conn_id: i32) -> Result<(), Error> {
        match self.tx.owner() {
            Some(owner) if owner != conn_id => Err(Error::WriterHeldElsewhere(format!(
                "could not begin or end a transaction block on connection {conn_id}: connection {owner}'s block is open, and an embedded database runs one block at a time"
            ))),
            _ => Ok(()),
        }
    }

    /// Carry a transaction command's outcome to the session's settings
    /// ([`TransactionSession::carry_to_settings`]) — `was` the block's status before it —
    /// answering the command's own error first.
    fn after_block_command(
        &mut self,
        conn_id: i32,
        command: &TransactionCommand,
        was: TransactionStatus,
        result: Result<(), Error>,
    ) -> Result<(), Error> {
        let carried =
            self.tx
                .carry_to_settings(self.settings.of_mut(conn_id), command, was, result.is_ok());
        result.and(carried)
    }

    /// Enter the facade for one call: the refusal of every call once the store has failed
    /// a durable write ([`StorageBackend::check_writable`], `57P02`), the byte check's entry
    /// (`pglet_sql::depth::enter_stack`), then the refusal of a catalog this thread could
    /// not walk — `54001` before anything of the call runs, and with no effect on the
    /// session, the storage or the transaction block
    /// (`pglet_sql::depth::check_catalog_walks`).
    ///
    /// Every public entry that runs SQL, commits, rolls back, persists, forks or reads
    /// the catalog out calls it first. A catalog is built under the bounds of the thread
    /// that ran its DDL; used where they are smaller — a count lowered since
    /// ([`set_thread_max_depth`]), a narrower budget ([`Self::set_stack_budget`]), a
    /// smaller thread, or a caller already deep in its own — a `BEGIN`'s snapshot of it
    /// overflowed the stack and aborted the process, and a commit's encoding refused a
    /// write it had already made. Refused here, the call can be made again from a
    /// thread that walks the catalog.
    fn enter(&self) -> Result<pglet_sql::depth::StackEntry, Error> {
        // First: after a failed durable write nothing is read or written, whatever the
        // entry — the in-memory state may hold a commit the durable state does not.
        self.storage.check_writable()?;
        let entry = pglet_sql::depth::enter_stack(self.stack_budget);
        self.catalog_depth.check(&self.schema)?;
        // The call is this host's client message: what it runs reads a statement
        // start of its own (`pglet_sql::builtins::start_statement`).
        pglet_sql::builtins::start_statement();
        Ok(entry)
    }

    /// Borrow the storage backend.
    pub const fn storage(&self) -> &StorageBackend {
        &self.storage
    }

    /// Mutably borrow the storage backend.
    ///
    /// A door a write can stage through, so with a block open it is the block's, as
    /// [`Self::schema_mut`] is: the block takes the writer first.
    pub fn storage_mut(&mut self) -> &mut StorageBackend {
        if let Some(owner) = self.tx.owner() {
            self.tx
                .take_writer_for_host(owner, &self.schema, &mut self.storage);
        }
        &mut self.storage
    }

    /// Parse, plan, and execute SQL, returning the result of each statement.
    ///
    /// This mirrors the engine half of the server's query path: parse → plan →
    /// `execute_with_storage`, with the aborted-block gate in front and the
    /// transaction-command dispatch behind, and no wire encoding. `BEGIN` / `COMMIT`
    /// / `ROLLBACK` written as SQL therefore open and end a real transaction here, as
    /// they do on the server and on the wasm bindings; [`Database::begin`] /
    /// [`Database::commit`] / [`Database::rollback`] drive the same session directly
    /// for a caller that would rather not go through SQL.
    ///
    /// Uses `conn_id` `0`; for multiple logical connections use `execute_as`.
    ///
    /// # Errors
    ///
    /// Returns the parser/planner/executor error on the first failing statement, or
    /// [`Error::AbortedTransaction`] (`25P02`) for a statement issued after one
    /// failed inside a transaction block.
    pub fn execute(&mut self, sql: &str) -> Result<Vec<QueryResult>, Error> {
        self.execute_as(sql, 0)
    }

    /// Like `execute`, but with an explicit `conn_id` for transaction routing.
    ///
    /// The transaction block belongs to the `conn_id` that opened it: a statement on
    /// any other logical connection neither reads its aborted state nor accumulates
    /// into its rollback frame (see [`TransactionSession::owns`]). This host holds one
    /// transaction session and runs on one thread, so what the native server would make
    /// wait it refuses `55P03`: another connection's `BEGIN`, `COMMIT` or `ROLLBACK`
    /// while a block is open, another connection's write while the block holds the
    /// writer, and another connection's statement whose plan fails because of a change
    /// that block made to the catalog (the native server makes it wait and plan again; a
    /// plan failing for its own reason answers its own error). Its reads
    /// otherwise run, and so does its write while the block has only read — the block's
    /// first write then re-bases it on that write. On a database built for one connection
    /// ([`Self::with_btree_for_one_connection`]) another connection's `BEGIN` and write
    /// are refused `0A000` where no refusal above answers first.
    ///
    /// # Errors
    ///
    /// Returns the parser/planner/executor error on the first failing statement,
    /// [`Error::AbortedTransaction`] (`25P02`) for a statement issued after one
    /// failed inside this connection's transaction block, or
    /// [`Error::WriterHeldElsewhere`] (`55P03`) or [`Error::NotImplemented`] (`0A000`) for
    /// the refusals above.
    pub fn execute_as(&mut self, sql: &str, conn_id: i32) -> Result<Vec<QueryResult>, Error> {
        let _stack = self.enter()?;
        let mut results = Vec::new();
        // A text with no `;` byte, outside an aborted block, is parsed first, as the one
        // piece the split below would cut, and run the long way only if its parse fails
        // (`raw_parse_can_wait`).
        if self.raw_parse_can_wait(conn_id, sql) {
            // What the session-command interception below does for a statement it does
            // not service.
            self.settings.of_mut(conn_id).install_ambient();
            let piece = pglet_sql::lexer::trim_space(sql);
            if let Ok(parsed) = pglet_sql::parser::parse_single(piece, &self.schema) {
                if let Some(planned) = parsed {
                    results.push(self.run_statement(&planned, conn_id)?);
                }
                return Ok(results);
            }
        }
        // `split_statements` walks the lexer so a `;` inside a string / dollar-quote
        // doesn't split. Its own error aborts an open block, exactly as a per-statement
        // one does — the block's state must not depend on which pass raised.
        #[cfg(test)]
        tests::count_raw_parse();
        let statements = match pglet_sql::parser::split_statements(sql) {
            Ok(statements) => statements,
            Err(e) => return Err(self.abort_block_with(conn_id, e)),
        };
        // Each statement is planned against the *current* schema (an earlier
        // statement's DDL is visible to a later one), then executed and persisted by
        // the shared `run_planned`.
        for stmt_sql in statements {
            match self.tx.aborted_block_action(conn_id, stmt_sql) {
                AbortedBlockAction::Refuse => return Err(Error::AbortedTransaction),
                AbortedBlockAction::ImplicitRollback => {
                    // `COMMIT`/`END` in an aborted block is Postgres's implicit
                    // rollback: no error, the block ends, and the reported command tag
                    // is `ROLLBACK` — nothing is parsed or executed. It runs as the
                    // session's commit of an aborted block, which is that rollback
                    // answered as the commit the client asked for.
                    self.commit_block(conn_id)?;
                    results.push(QueryResult::ddl("ROLLBACK"));
                    continue;
                }
                AbortedBlockAction::Run => {}
            }
            // The session-hygiene statements run ahead of the planner on **every**
            // host, through the one dispatcher — `SET`, `SHOW`, `DEALLOCATE`,
            // `DISCARD`, `RESET`. Written per host they drifted: the native server
            // serviced `SET`/`SHOW` while this leg answered the parser's `0A000` for
            // both, so a driver's connection-setup batch worked over the socket and
            // failed in-process against the same engine.
            //
            // `DEALLOCATE`/`DISCARD ALL`'s prepared-statement namespace is the one
            // thing this host does not have: the embed API (and the wasm bindings
            // over it) bind parameter values directly and mint no statement handle.
            // That is not a case to special-case around — `&mut ()` is an empty
            // namespace, which answers exactly what 18.6 answers for a session that
            // has prepared nothing, so `ALL` succeeds and a name is `26000`.
            let in_block = self.tx.owns(conn_id) && self.tx.status() != TransactionStatus::Idle;
            if let Some(outcome) = pglet_sql::catalog::execute_session_command(
                stmt_sql,
                &mut (),
                &mut self.sequence_session,
                self.settings.of_mut(conn_id),
                in_block,
            ) {
                match outcome {
                    Ok(result) => results.push(result),
                    Err(e) => return Err(self.abort_block_with(conn_id, e)),
                }
                continue;
            }
            let parsed = match pglet_sql::parser::parse_single(stmt_sql, &self.schema) {
                Ok(parsed) => parsed,
                Err(e) => {
                    let e = self.plan_refusal(conn_id, e, |committed| {
                        pglet_sql::parser::parse_single(stmt_sql, committed)
                    });
                    return Err(self.abort_block_with(conn_id, e));
                }
            };
            if let Some(planned) = parsed {
                results.push(self.run_statement(&planned, conn_id)?);
            }
        }
        Ok(results)
    }

    /// Run one planned statement of [`Self::execute_as`] and apply the transaction
    /// lifecycle it calls for, a failure of either aborting the block.
    fn run_statement(&mut self, planned: &Plan, conn_id: i32) -> Result<QueryResult, Error> {
        let result = match self.run_planned(planned, conn_id) {
            Ok(result) => result,
            Err(e) => return Err(self.abort_block_with(conn_id, e)),
        };
        if let Some(command) = result.tx_command.clone() {
            // A lifecycle failure aborts the block like any other statement error: a
            // `ROLLBACK TO` naming no live savepoint is `3B001` **and leaves the block
            // aborted** on 18.6, and a route that skipped the abort would answer the next
            // statement normally and then commit.
            if let Err(e) = self.apply_transaction_command(conn_id, &command) {
                return Err(self.abort_block_with(conn_id, e));
            }
        }
        Ok(result)
    }

    /// The transaction block's current state — `Idle`, `InTransaction`, or the
    /// aborted `Failed`.
    #[must_use]
    pub const fn transaction_status(&self) -> TransactionStatus {
        self.tx.status()
    }

    /// How many transaction levels are open: `0` when idle, `1` inside a
    /// `BEGIN … COMMIT`, and one more per live savepoint.
    ///
    /// The **engine's own count**, which is what a host driving this database
    /// should read rather than mirroring the block state off command tags:
    /// `ROLLBACK TO SAVEPOINT` reports tag `ROLLBACK`, so a tag-derived mirror
    /// believes the whole block ended and lets the next `BEGIN` open a second one
    /// over the first. The npm adapter reads this for exactly that reason.
    #[must_use]
    pub const fn transaction_depth(&self) -> usize {
        self.tx.frame_depth()
    }

    /// Whether `sql` can be parsed ahead of its raw parse — the whole-text lex that
    /// counts its statements — the aborted-block gate and the session-command
    /// interception, and taken the long way through all three only if that parse fails.
    ///
    /// 18.6's raw parse comes first (`exec_simple_query`, `exec_parse_message`): it
    /// counts the statements, and a lexical error is the statement's answer ahead of the
    /// gate, a session command and every error parse analysis raises. A text with no `;`
    /// byte holds at most one statement, so the count is known. Outside an aborted block
    /// the gate runs every statement. A parse that succeeds has lexed the whole text, so
    /// no lexical error was there to answer; and it is no session command, every one of
    /// which the parser refuses (`pglet_sql::catalog::is_session_command`). So a parse
    /// that succeeds is the answer the long way would reach, and one that fails left
    /// nothing behind but the ambient session it installed, which the long way installs
    /// again. In an aborted block the gate answers `25P02` where
    /// a lexical error answers `42601`, so the raw parse comes first there, as it does
    /// for a text with a `;`.
    fn raw_parse_can_wait(&self, conn_id: i32, sql: &str) -> bool {
        !sql.bytes().any(|b| b == b';')
            && (!self.tx.owns(conn_id) || self.tx.status() != TransactionStatus::Failed)
    }

    /// Abort this connection's transaction block (if it has one open) and hand the
    /// error back unchanged.
    ///
    /// Every route out of a statement — the split, the parse/plan, the execution —
    /// goes through this, because Postgres aborts the block on *any* statement error
    /// and a route that skips it leaves a block that answers the next statement
    /// normally and then commits.
    fn abort_block_with(&mut self, conn_id: i32, error: Error) -> Error {
        self.tx.fail(conn_id, &mut self.storage);
        error
    }

    /// Apply the transaction lifecycle an executed statement calls for.
    fn apply_transaction_command(
        &mut self,
        conn_id: i32,
        command: &TransactionCommand,
    ) -> Result<(), Error> {
        let was = self.tx.status();
        let Self {
            schema,
            storage,
            tx,
            ..
        } = self;
        let result = tx.apply_command(conn_id, command, schema, storage);
        self.after_block_command(conn_id, command, was, result)
    }

    /// Commit the block through the session — for an aborted block, its rollback,
    /// answered as a commit ([`TransactionSession::commit`]).
    fn commit_block(&mut self, conn_id: i32) -> Result<(), Error> {
        let was = self.tx.status();
        let Self {
            schema,
            storage,
            tx,
            ..
        } = self;
        let result = tx.commit(conn_id, schema, storage);
        self.after_block_command(conn_id, &TransactionCommand::Commit, was, result)
    }

    /// Roll the block back through the session (its unconditional restore included).
    fn rollback_block(&mut self, conn_id: i32) -> Result<(), Error> {
        let was = self.tx.status();
        let Self {
            schema,
            storage,
            tx,
            ..
        } = self;
        let result = tx.rollback(conn_id, schema, storage);
        self.after_block_command(conn_id, &TransactionCommand::Rollback, was, result)
    }

    /// Parse, bind positional `$N` parameters, then execute and persist a single
    /// statement — the param-aware sibling of [`Database::execute_as`].
    ///
    /// The plan is **never mutated to bind**: the inferred parameter types are
    /// handed to `bind`, which decodes the host's own argument representation
    /// (e.g. JS values) into engine [`Value`]s; the bound values then ride an
    /// `EvalEnv` the executor resolves at eval time, exactly like the server's
    /// extended protocol. `sql` must be exactly one statement.
    ///
    /// A statement's plan and its parameters' inferred types may be held, keyed by the
    /// text, and a later call with the same text reuses them in place of the parse, the
    /// statement count and the inference while the catalog's [`Schema::generation`] and
    /// the calling thread's planner settings stand; everything else — the aborted-block
    /// gate, the session commands, binding and execution — runs on a hit as on a miss.
    /// What is held is bounded, short texts only, and never a statement that changes what
    /// a parse reads; it is emptied when the generation or those settings move, by
    /// [`Self::schema_mut`] and by [`Self::set_stack_budget`], and a [`Self::fork`]
    /// starts with none. Natively a reused plan is not refused `54001` where its parse
    /// would be — a call from deeper in the caller's stack, or one made inside another
    /// database's `bind` callback — and its execution's own checks still run.
    ///
    /// `bind` receives the inferred [`TypeOid`] of each `$N` (`None` where
    /// inference couldn't pin a type) and returns one `Option<Value>` per
    /// parameter (`None` = SQL `NULL`); an empty `Vec` runs the statement with no
    /// bound parameters. Keeping decoding in the callback lets a host (e.g. the
    /// wasm bindings) own its value marshaling without the engine depending on it.
    ///
    /// # Errors
    ///
    /// Returns [`BoundExecError::Engine`] for a parse / plan / execute error (a
    /// statement count other than one included, and [`Error::AbortedTransaction`] for
    /// a statement issued inside an aborted block) and [`BoundExecError::Bind`] if
    /// `bind` fails. Either fails the connection's open block.
    pub fn execute_bound<F, E>(
        &mut self,
        sql: &str,
        conn_id: i32,
        bind: F,
    ) -> Result<QueryResult, BoundExecError<E>>
    where
        F: FnOnce(&[Option<TypeOid>]) -> Result<Vec<Option<Value>>, E>,
    {
        let _stack = self.enter()?;
        // A text with no `;` byte, outside an aborted block, is parsed first, and run
        // the long way below only if its parse fails (`raw_parse_can_wait`).
        let mut waited = self.raw_parse_can_wait(conn_id, sql);
        let prepared = loop {
            if waited {
                // What the session-command interception below does for a statement it
                // does not service.
                self.settings.of_mut(conn_id).install_ambient();
            } else {
                // The single-statement check first, then the gate — `exec_parse_message`'s
                // order (raw parse, `cannot insert multiple commands into a prepared
                // statement`, *then* the abort test), and this method's own contract
                // besides. Behind the gate it would make `COMMIT; SELECT 1` in an aborted
                // block answer tag `ROLLBACK` and silently drop the second statement
                // instead of refusing the pair. Counted as 18.6 counts them: a piece of
                // comments alone is no statement.
                match raw_statements(sql) {
                    Ok(statements) if statements.len() > 1 => {
                        let e = Error::syntax(format!(
                            "expected 1 statement, got {}",
                            statements.len()
                        ));
                        return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e)));
                    }
                    Ok(_) => {}
                    Err(e) => {
                        return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e)));
                    }
                }
                // The aborted-block gate runs ahead of parse analysis, where Postgres puts
                // it, so a refused statement is never analyzed and a `COMMIT` never reaches
                // the parser at all — it is converted to the rollback Postgres reports as
                // tag `ROLLBACK`.
                match self.tx.aborted_block_action(conn_id, sql) {
                    AbortedBlockAction::Refuse => {
                        return Err(BoundExecError::Engine(Error::AbortedTransaction));
                    }
                    AbortedBlockAction::ImplicitRollback => {
                        self.commit_block(conn_id).map_err(BoundExecError::Engine)?;
                        return Ok(QueryResult::ddl("ROLLBACK"));
                    }
                    AbortedBlockAction::Run => {}
                }
                // The same session-command interception `execute_as` makes, at this host's
                // other entry point. A session-hygiene statement takes no parameters, so
                // `bind` is never called for one — but reaching this method with a `SHOW`
                // must not answer the parser's `0A000` where the other entry point answers
                // the setting.
                let in_block = self.tx.owns(conn_id) && self.tx.status() != TransactionStatus::Idle;
                if let Some(outcome) = pglet_sql::catalog::execute_session_command(
                    sql,
                    &mut (),
                    &mut self.sequence_session,
                    self.settings.of_mut(conn_id),
                    in_block,
                ) {
                    return outcome
                        .map_err(|e| BoundExecError::Engine(self.abort_block_with(conn_id, e)));
                }
            }
            // A statement parsed before, under the catalog and the settings this one is
            // parsed under, is that parse's product (`plan_cache`). It stands in for the
            // parse, the statement count and the inference below, and for nothing above:
            // the raw parse, the gate, the session-command interception and the ambient
            // session's install run for it as for any statement.
            if let Some(prepared) = self.plan_cache.get(sql, &self.schema) {
                break prepared;
            }
            // One parse: inference reads this plan, `bind` decodes against its inferred
            // OIDs, and execution runs the SAME immutable plan — no re-parse, no plan
            // mutation to bind.
            let mut plans = match pglet_sql::parser::parse(sql, &self.schema) {
                Ok(plans) => plans,
                Err(_) if waited => {
                    waited = false;
                    continue;
                }
                Err(e) => {
                    let e = self.plan_refusal(conn_id, e, |committed| {
                        pglet_sql::parser::parse(sql, committed)
                    });
                    return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e)));
                }
            };
            if plans.len() != 1 {
                let e = Error::syntax(format!("expected 1 statement, got {}", plans.len()));
                return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e)));
            }
            let plan = plans.remove(0);
            // No `$` in the text is no `$N` in the plan: nothing to infer, and the walk —
            // one that cannot take the byte check — is not run over a tree it has nothing
            // to do in (`pglet_sql::depth`).
            let targets = if sql.contains('$') {
                // A parameter parse analysis leaves without a type is the statement's
                // refusal, raised where 18.6 raises it: before any value is bound.
                match infer_param_targets(&plan, &self.schema, &[]) {
                    Ok(targets) => targets,
                    Err(e) => {
                        return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e)));
                    }
                }
            } else {
                pglet_sql::planner::ParamTargets::default()
            };
            break self.plan_cache.insert(sql, Prepared { plan, targets });
        };
        let Prepared { plan, targets } = &*prepared;
        let oids = &targets.types;
        // A binding failure is the host's error, but it fails an open block all the
        // same: 18.6 aborts the transaction on an error raised at `Bind` as it does on
        // one raised at `Parse` or `Execute`, and the native server's extended protocol
        // does too. The error itself
        // passes through in the host's own form.
        let bound = match bind(oids) {
            Ok(bound) => bound,
            Err(e) => {
                self.tx.fail(conn_id, &mut self.storage);
                return Err(BoundExecError::Bind(e));
            }
        };
        // A bound `Value` arrives already built, so no reader or arithmetic producer
        // judged it: a `date` or `timestamp` outside `0001-01-01 ..= 9999-12-31` is
        // refused here rather than stored and dumped as text no Postgres reads back, and
        // a `timestamp` is brought to UTC, the offset every renderer and cast assumes, and
        // to a whole microsecond, the precision the store persists.
        let bound = match bound
            .into_iter()
            .map(|v| v.map(pglet_core::types::normalize_value_domain).transpose())
            .collect::<Result<Vec<_>, _>>()
        {
            Ok(bound) => bound,
            Err(e) => return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e))),
        };
        // The enum conversion of a bound value, where 18.6 runs it: at `Bind`, on the
        // value, once — not at eval time on a row. A non-member label is `22P02` here
        // over an empty table and inside a `WHERE false` alike, both measured on 18.6,
        // and it is a *write* that made this a gate rather than a nicety: without it a
        // `DELETE … WHERE m < $1` removed every row and an `INSERT … SELECT
        // greatest(m, $1)` landed non-members in a `text` column.
        if let Err(e) = bound_param_enum_legality(&targets.enums, &bound, &self.schema) {
            return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e)));
        }
        let env = if bound.is_empty() {
            EvalEnv::EMPTY
        } else {
            EvalEnv::with_params(&bound)
        };
        let result = match self.run_planned_env(plan, conn_id, env) {
            Ok(result) => result,
            Err(e) => return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e))),
        };
        if let Some(command) = result.tx_command.clone()
            && let Err(e) = self.apply_transaction_command(conn_id, &command)
        {
            return Err(BoundExecError::Engine(self.abort_block_with(conn_id, e)));
        }
        Ok(result)
    }

    /// Execute one already-planned statement against the live schema/storage and
    /// persist an auto-commit write. Shared by both parser front-ends.
    fn run_planned(&mut self, planned: &Plan, conn_id: i32) -> Result<QueryResult, Error> {
        self.run_planned_env(planned, conn_id, EvalEnv::EMPTY)
    }

    /// Execute one already-planned statement under a caller-supplied `EvalEnv`
    /// (bound `$N` parameters and/or an outer-scope frame) and persist an
    /// auto-commit write. The shared execute+persist core behind `run_planned`
    /// (no params) and `execute_bound` (bound params).
    fn run_planned_env(
        &mut self,
        planned: &Plan,
        conn_id: i32,
        env: EvalEnv<'_>,
    ) -> Result<QueryResult, Error> {
        // The `25001` gate — `CREATE`/`DROP INDEX CONCURRENTLY` inside a transaction
        // block. Asked of the plan (`Plan::forbidden_in_transaction_block`, the one
        // rule) at this host's single execute-one-statement choke point, which is
        // where the block state lives; the server's `run_one` asks the same question
        // at its own.
        if self.tx.owns(conn_id)
            && let Some(op) = planned.forbidden_in_transaction_block()
        {
            return Err(Error::ActiveSqlTransaction { op });
        }
        if matches!(
            planned,
            Plan::BeginTransaction { .. } | Plan::CommitTransaction | Plan::RollbackTransaction
        ) {
            self.refuse_foreign_block_control(conn_id)?;
        }
        // A statement the block runs takes its snapshot, the first fixing its isolation
        // level; a first write's is this moment's
        // (`TransactionSession::note_snapshot`).
        if !planned.is_transaction_control() {
            let first_write_at = planned.takes_writer().then(|| self.storage.commit_stamp());
            self.tx
                .note_snapshot(conn_id, self.settings.of_mut(conn_id), first_write_at);
        }
        // Before anything stages: the writer's refusal or its re-base.
        if planned.takes_writer() {
            self.claim_writer(conn_id)?;
        }
        // Auto-commit durability: a write applied outside an explicit transaction
        // must persist the meta root, or a restart loses it. The explicit-COMMIT
        // path persists via `commit`/`save_state`; auto-commit writes (the dominant
        // path) bypassed it. Persist the FULL metadata (incl. schema) here, at the
        // layer that has the schema, after the write lands.
        //
        // Gate on `mutates_storage()`, not `!is_read_only()`: transaction-control
        // statements are non-read-only yet touch no page store, so the finer
        // predicate avoids a pointless meta flip on a stray `BEGIN`/`COMMIT` plan.
        // The counters this statement is about to replace, read *before* it runs —
        // the value the outgoing cell holds is what a rollback puts back, and after
        // the statement it is gone. Outside this connection's own block the answer is
        // discarded.
        let sequence_swaps = if self.tx.owns(conn_id) {
            planned.swapped_sequences(&self.schema)
        } else {
            Vec::new()
        };

        let needs_persist = planned.mutates_storage() && !self.storage.has_transaction(conn_id);
        // Whether a write the device refuses at this statement's commit can drop its
        // staged writes and leave the database serving — asked before it runs.
        let discardable = needs_persist && autocommit_is_discardable(planned, &self.schema);
        if !needs_persist {
            // A read, a statement already inside an explicit transaction (whose COMMIT
            // persists), or a transaction-control plan: run directly, no wrap.
            let result = {
                let mut adapter = StorageAdapter::new(&mut self.storage, conn_id);
                execute_with_storage(
                    planned,
                    &mut self.schema,
                    &mut adapter,
                    env,
                    &mut self.sequence_session,
                )
            };
            self.tx.record_statement(conn_id, sequence_swaps, &result);
            return result;
        }

        // Wrap the auto-commit write in an implicit storage transaction so a multi-row
        // statement (INSERT/UPDATE/COPY) that violates a constraint partway through
        // rolls back its earlier rows atomically — Postgres's per-statement atomicity —
        // instead of leaving them applied. With a transaction active for this conn_id,
        // `StorageAdapter` routes every insert/update to the pending buffer, so the rows
        // stage and publish in one CoW root swap at commit, exactly like an explicit
        // BEGIN (read-your-own-writes within the statement stays correct via the
        // `*_in_tx` read paths). For the memory backend `begin`/`commit` are no-ops and
        // `has_transaction` is always false, so this bracket is a no-op there and the
        // memory autocommit path is unchanged.
        self.storage.begin(conn_id)?;
        let result = {
            let mut adapter = StorageAdapter::new(&mut self.storage, conn_id);
            execute_with_storage(
                planned,
                &mut self.schema,
                &mut adapter,
                env,
                &mut self.sequence_session,
            )
        };
        self.tx.record_statement(conn_id, sequence_swaps, &result);
        match result {
            Ok(r) => {
                self.publish_autocommit(conn_id, discardable)?;
                Ok(r)
            }
            Err(e) => {
                // Discard the staged writes; surface the ORIGINAL error (a rollback
                // failure must not mask it).
                let _ = self.storage.rollback(conn_id);
                Err(e)
            }
        }
    }

    /// Publish an auto-commit write: encode the catalog where the store keeps one, then
    /// commit the staged writes, then persist the meta root beside them. Shared by the
    /// SQL write path (`run_planned_env`) and `copy_in`.
    ///
    /// The encoding runs first, before the staged writes publish ([`autocommit_publish_bytes`]).
    /// The entry's check ([`Self::enter`]) is what keeps it from refusing
    /// on a catalog's depth; where it refuses anyway, the staged writes are discarded
    /// with it.
    ///
    /// Two steps on a durable store: the storage commit routes through `apply_batch`,
    /// which writes the staged data pages and fsyncs them (nothing, where no page
    /// changed); `save_state` then writes the catalog the same way where it changed
    /// (`persist_catalog_if_changed`), and the meta slot with the full metadata, fsynced
    /// — the durable commit point. So a write that changes rows and no catalog takes two
    /// fsyncs here, and one that also changes the catalog three. A failure of either step
    /// is an I/O failure past the encoding, and is returned as it happened.
    /// `discardable` ([`autocommit_is_discardable`]) lets a write the device refuses
    /// before anything is published drop the staged writes and leave the database
    /// serving (`Error::WriteRefused`).
    fn publish_autocommit(&mut self, conn_id: i32, discardable: bool) -> Result<(), Error> {
        let schema_bytes = autocommit_publish_bytes(conn_id, &self.schema, &mut self.storage)?;
        if discardable {
            self.storage.commit_discardable(conn_id)?;
        } else {
            self.storage.commit(conn_id)?;
        }
        self.storage.save_state(schema_bytes).map_err(Error::from)
    }

    /// Persist the full metadata (schema + storage root) on demand — [`Self::sync`]'s
    /// second half.
    fn persist_autocommit(&self) -> Result<(), Error> {
        let schema_bytes = transaction::catalog_bytes(&self.schema, &self.storage)?;
        self.storage.save_state(schema_bytes)?;
        Ok(())
    }

    /// Execute `COPY <table> [(cols)] TO STDOUT` (text format), returning the encoded
    /// COPY bytes.
    ///
    /// The byte stream is out-of-band — it can't ride the `QueryResult` channel — so
    /// COPY is driven through this method rather than `execute`. `sql` must be a
    /// single `COPY … TO STDOUT` statement; a `COPY … FROM STDIN` or any other
    /// statement fails loud. This is a read against the committed/auto-commit
    /// snapshot.
    ///
    /// # Errors
    ///
    /// Returns the parser error, an error if `sql` is not a `COPY … TO STDOUT`, or the
    /// executor error (`0A000` non-text format, `42P01`/`42703` missing table/column,
    /// `42701` a column the list names twice); [`Error::AbortedTransaction`] (`25P02`) inside
    /// connection 0's aborted block; [`Error::WriterHeldElsewhere`] (`55P03`) when another
    /// connection's block holds the writer and its catalog change is why the table or column
    /// list fails to resolve.
    pub fn copy_out(&mut self, sql: &str) -> Result<Vec<u8>, Error> {
        const WHAT: &str = "copy_out requires a COPY … TO STDOUT statement";
        let _stack = self.enter()?;
        // **Inert today**, and installed for symmetry with `copy_in` rather than for an
        // effect: `execute_copy_to` evaluates no expression at all (table → columns →
        // scan → text encode), so a `COPY … TO STDOUT` has nothing to resolve against
        // a session. It keeps this facade's two COPY entry points from differing on a
        // rule neither of them should have to restate.
        self.settings.of(0).install_ambient();
        // COPY is a statement like any other as far as the block is concerned, and it
        // arrives here rather than through `execute` only because its bytes need an
        // out-of-band channel. The native server's `try_copy_*` interceptors defer to
        // `execute_query_core`'s gate for exactly this; without the same test here the
        // embed/wasm leg would run a COPY inside an aborted block that every other
        // route refuses. A `COPY` is never an exit statement, so no classification is
        // needed — only the state.
        if self.tx.owns(0) && self.tx.status() == TransactionStatus::Failed {
            return Err(Error::AbortedTransaction);
        }
        let plan = pglet_sql::parser::parse_single(sql, &self.schema)?
            .ok_or_else(|| Error::Execution(WHAT.into()))?;
        let params = copy_params_of(&plan, CopyDirection::To, WHAT)?;
        // The block's snapshot, as `run_planned_env` takes it: the first fixes the block's
        // isolation level.
        self.tx.note_snapshot(0, self.settings.of_mut(0), None);
        // The table and column list, which the plan leaves to execution, resolved as a
        // plan is: a failure another connection's uncommitted catalog change caused is
        // refused as that plan's is (`plan_refusal`).
        if let Err(e) = copy_column_count(&self.schema, &params) {
            return Err(self.plan_refusal(0, e, |committed| copy_column_count(committed, &params)));
        }
        execute_copy_to(&self.storage, &self.schema, &params)
    }

    /// Execute `COPY <table> [(cols)] FROM STDIN` (text format), ingesting `data` and
    /// returning the inserted row count.
    ///
    /// `sql` must be a single `COPY … FROM STDIN` statement; anything else fails loud.
    /// The ingest runs the same validation/constraint/FK/index path as `INSERT`. With no
    /// block open on connection 0 it commits and persists as an auto-commit `INSERT` via
    /// `execute` does, so a `copy_in` survives a restart; inside that connection's block
    /// it stages with the block's other writes, the block's `COMMIT` persisting it and a
    /// failure aborting the block.
    ///
    /// # Errors
    ///
    /// Returns the parser error, an error if `sql` is not a `COPY … FROM STDIN`, or the
    /// executor error (`22P04` framing, `22021` encoding, `22P02` value, `23502`/`23505`/`23503`
    /// constraint, `0A000` non-text format, `42P01`/`42703` missing table/column,
    /// `42701` a column the list names twice); [`Error::AbortedTransaction`] (`25P02`)
    /// inside connection 0's aborted block; [`Error::WriterHeldElsewhere`] (`55P03`) while
    /// another connection's block holds the writer, or when that block's catalog change
    /// is why the table or column list fails to resolve.
    pub fn copy_in(&mut self, sql: &str, data: &[u8]) -> Result<u64, Error> {
        const WHAT: &str = "copy_in requires a COPY … FROM STDIN statement";
        let _stack = self.enter()?;
        // The ambient session this statement's stored expressions resolve against —
        // the embed facade's COPY site, enumerated with the others on
        // `SessionSettings::install_ambient`, which is the one home for that list.
        // COPY reaches this facade through its own entry point rather than
        // `execute_as`/`execute_bound`, so it misses the install those make through
        // `catalog::execute_session_command` — the same shape the native server's COPY
        // interceptors have, where it is a live wrong-row defect
        // (`pglet_server::copy`'s `ambient_session_tests`).
        //
        // **Unobservable here today, and only by accident of the facade's API**:
        // every embed/wasm host answers the `DEFAULT_SESSION_NAME` constant, because
        // `Database` exposes no way to set an identity — `SessionSettings::with_identity`
        // exists and only the native server calls it. The day this facade takes one,
        // the omission is the server's defect with no other code change, so the line
        // goes in with the reason rather than after.
        self.settings.of(0).install_ambient();
        let conn_id = 0;
        // The aborted-block gate, for the reason on `copy_out`. A failure below aborts
        // the block too — a COPY that raises partway is a statement that raised.
        if self.tx.owns(conn_id) && self.tx.status() == TransactionStatus::Failed {
            return Err(Error::AbortedTransaction);
        }
        let plan = pglet_sql::parser::parse_single(sql, &self.schema)?
            .ok_or_else(|| Error::Execution(WHAT.into()))?;
        let params = copy_params_of(&plan, CopyDirection::From, WHAT)?;
        // The block's snapshot, as `run_planned_env` takes it: this is a write.
        let first_write_at = Some(self.storage.commit_stamp());
        self.tx
            .note_snapshot(conn_id, self.settings.of_mut(conn_id), first_write_at);
        // The table and column list, resolved as `copy_out` resolves them — before the
        // writer, as the native server resolves them before `CopyInResponse`.
        if let Err(e) = copy_column_count(&self.schema, &params) {
            let e = self.plan_refusal(conn_id, e, |committed| {
                copy_column_count(committed, &params)
            });
            return Err(self.abort_block_with(conn_id, e));
        }
        // A door that stages writes: the writer's refusal or its re-base, before any.
        if let Err(e) = self.claim_writer(conn_id) {
            return Err(self.abort_block_with(conn_id, e));
        }
        // Wrap an auto-commit COPY ingest in an implicit storage transaction so a bad
        // line partway through the stream rolls back the already-ingested lines
        // atomically (Postgres COPY is all-or-nothing) instead of leaving them applied.
        // Inside an explicit transaction the ingest stays buffered and the COMMIT
        // persists it, so only the auto-commit case wraps + persists here.
        let autocommit = !self.storage.has_transaction(conn_id);
        if autocommit {
            self.storage.begin(conn_id)?;
        }
        let count = {
            let mut adapter = StorageAdapter::new(&mut self.storage, conn_id);
            execute_copy_from(&mut adapter, &mut self.schema, &params, data)
        };
        match count {
            Ok(n) => {
                if autocommit {
                    // A `COPY … FROM` writes no catalog fact but the sequence counters
                    // a column default advances, which no rollback puts back — so a
                    // refused write can drop its rows.
                    self.publish_autocommit(conn_id, true)?;
                }
                Ok(n)
            }
            Err(e) => {
                if autocommit {
                    let _ = self.storage.rollback(conn_id);
                }
                Err(self.abort_block_with(conn_id, e))
            }
        }
    }

    /// Emit a portable SQL dump of the whole database — every `CREATE TYPE` /
    /// `CREATE SEQUENCE` / `CREATE TABLE`, the table data as `COPY … FROM stdin`
    /// blocks, then the `PRIMARY KEY` / `UNIQUE` / index / `CHECK` / `FOREIGN KEY`
    /// definitions — whose target is to recreate the database on both stock Postgres
    /// and pglet, the dump invariant `docs/sequences.md` states, which `EDGES.md`
    /// records databases falling short of.
    ///
    /// A pure read of the committed/auto-commit snapshot: neither the schema nor the
    /// storage is mutated. Reuses the same `COPY` text encoder the wire path uses for
    /// the data section.
    ///
    /// # Errors
    ///
    /// Returns an error if any catalog element can't be rendered to round-trippable
    /// SQL (an unsupported column type, an unreproducible `DEFAULT`, a non-expressible
    /// identifier, or a `CHECK` / index expression outside the rendered subset), a
    /// storage error from scanning a table, `Error::InvalidEncoding` (`22021`) when
    /// a table's `COPY` text or a stored jsonb string holds a NUL or a byte that is not
    /// valid UTF-8, `Error::TemporalOutOfRange` (`22008`) when a stored `date` or
    /// `timestamp` is outside the date domain, or `Error::Execution` (`22000`) when a
    /// stored `timestamp` is at a non-UTC offset.
    pub fn dump_sql(&self) -> Result<String, Error> {
        let _stack = self.enter()?;
        dump::dump_sql(&self.schema, &self.storage)
    }

    /// Begin a transaction for `conn_id` — the same session a SQL `BEGIN` opens.
    ///
    /// Snapshots what a rollback will restore: the catalog on every backend, and the
    /// whole `MemoryStorage` on the backend that buffers no pending writes of its
    /// own. A `BEGIN` while a block is already open is a no-op, keeping the first
    /// block's baseline (see [`TransactionSession::begin`]).
    ///
    /// # Errors
    ///
    /// `55P03` when another connection's block is open — this `Database` holds one
    /// transaction session — `0A000` on a database built for another connection
    /// ([`Self::with_btree_for_one_connection`]), or a storage error if the backend can't
    /// start the transaction.
    pub fn begin(&mut self, conn_id: i32) -> Result<(), Error> {
        let _stack = self.enter()?;
        self.refuse_foreign_block_control(conn_id)?;
        let was = self.tx.status();
        let Self {
            schema,
            storage,
            tx,
            ..
        } = self;
        let result = tx.begin(conn_id, schema, storage);
        self.after_block_command(conn_id, &TransactionCommand::Begin(None), was, result)
    }

    /// Commit the transaction for `conn_id`, atomically applying buffered writes,
    /// then persist the storage state, and the catalog beside it where the store keeps
    /// one ([`StorageBackend::persists_catalog`]).
    ///
    /// **A commit of an aborted block rolls it back instead** and publishes nothing —
    /// Postgres's implicit rollback, so this method and a SQL `COMMIT` agree (see
    /// [`TransactionSession::commit`]).
    ///
    /// Where the store keeps a catalog, it is encoded *before* the storage commit
    /// publishes anything, and a failure there rolls the block back and ends it, as a
    /// failing `COMMIT` does on 18.6 — never flipping the durable meta slot to an empty
    /// catalog. For memory storage the commit only counts itself
    /// ([`StorageBackend::commit_stamp`]) and the persist does nothing.
    ///
    /// # Errors
    ///
    /// `55P03` when another connection's block is open, which this does not touch; the
    /// encoding error (the block rolled back), or a storage error if the commit or the
    /// state persist fails.
    pub fn commit(&mut self, conn_id: i32) -> Result<(), Error> {
        let _stack = self.enter()?;
        self.refuse_foreign_block_control(conn_id)?;
        self.commit_block(conn_id)
    }

    /// Roll back the transaction for `conn_id`, discarding buffered writes **and
    /// restoring the catalog**.
    ///
    /// The restore is the session's: whole for a block that took the writer, nothing for
    /// one that never wrote ([`TransactionSession::rollback`]). The block always ends,
    /// even when the rollback fails — once past the entry check: on a store that has
    /// failed a durable write it answers `57P02` (`Error::DatabaseMustBeReopened`) and
    /// touches nothing.
    ///
    /// # Errors
    ///
    /// `55P03` when another connection's block is open, which this does not touch; a
    /// storage error if the backend rollback fails, or the error of the save that writes
    /// the restored catalog or the counters ([`TransactionSession::rollback`]).
    pub fn rollback(&mut self, conn_id: i32) -> Result<(), Error> {
        let _stack = self.enter()?;
        self.refuse_foreign_block_control(conn_id)?;
        self.rollback_block(conn_id)
    }

    /// Flush buffered page data and persist the storage root, and the catalog beside it
    /// where the store keeps one ([`StorageBackend::persists_catalog`]), on demand — the
    /// explicit form of the durability barrier an auto-commit write applies implicitly
    /// through `publish_autocommit`. Nothing is made durable on a backend without a
    /// persistent store (memory storage and the in-memory B+tree). The wasm OPFS host
    /// exposes this as its `sync`.
    ///
    /// # Errors
    ///
    /// Returns an error if the page flush, schema serialization, or metadata write
    /// fails.
    pub fn sync(&self) -> Result<(), Error> {
        let _stack = self.enter()?;
        self.storage.flush()?;
        self.persist_autocommit()
    }

    /// Re-register the catalog's persisted indexes with the storage backend after a
    /// reopen. **Both** the native (`open`) and wasm-OPFS reopen paths must call this.
    ///
    /// The index *entries* already live in the tree (they share the data root and
    /// survive a reopen), but the wrapper's in-memory def map — which drives index
    /// maintenance on writes and probe validation on reads — does not persist. Without
    /// this, a reopened indexed database errors on index probes and silently skips
    /// index maintenance on writes (drifting results). `reattach_index` does no
    /// back-fill (the entries are present).
    ///
    /// It first reconciles the store with the catalog
    /// (`StorageBackend::reconcile_with_catalog`): a table the tree's directory holds and
    /// the catalog does not name is forgotten, and the OID counter is seeded past every
    /// index id the tree holds an entry under — the catalog's greatest OID is below a
    /// number whose index a block dropped or took back while a save wrote its entries,
    /// and a new index handed that number read them as its own.
    ///
    /// For a **plain** index, resolves each column name to a row position against the
    /// restored table; for an **expression** index, carries the key parts as
    /// serialized `Expr` IR — the same shape `CREATE INDEX` produced. An index whose
    /// table or (plain) column can no longer be resolved is skipped: the catalog and
    /// tree are written atomically, so this shouldn't happen, but a stale def must not
    /// abort the reopen.
    ///
    /// # Errors
    ///
    /// Returns an error if the reconciliation cannot read the tree, building an index spec
    /// fails, or the backend rejects the re-registration.
    pub fn reattach_persisted_indexes(&mut self) -> Result<(), Error> {
        let _stack = self.enter()?;
        // It writes the directory and the OID counter: with a block open, the block's
        // writer first, as at every door that writes.
        if let Some(owner) = self.tx.owner() {
            self.tx
                .take_writer_for_host(owner, &self.schema, &mut self.storage);
        }
        self.storage.reconcile_with_catalog(&mut self.schema)?;
        // Build every spec first (borrows `schema`), then re-register (borrows
        // `storage`) — split so the two field borrows don't overlap.
        let mut specs = Vec::new();
        for index in self.schema.indexes.values() {
            let Some(table) = self.schema.get_table(&index.table_name) else {
                continue;
            };
            match index_spec_from_catalog(index, table, &self.schema.enums) {
                Ok(Some(spec)) => specs.push((index.name.clone(), spec)),
                Ok(None) => {}
                Err(e) => {
                    return Err(Error::Execution(format!(
                        "build index spec {}: {e}",
                        index.name
                    )));
                }
            }
        }
        for (name, spec) in specs {
            self.storage
                .reattach_index(spec)
                .map_err(|e| Error::Execution(format!("re-register index {name}: {e}")))?;
        }
        Ok(())
    }

    /// Set whether unsupported SQL types (NUMERIC, DATE, TIME, INTERVAL, and any
    /// unrecognized type) coerce to `TEXT` instead of erroring.
    ///
    /// Default `false`: an unsupported type in a `CAST`/`::` or a `CREATE
    /// TABLE`/`ALTER TABLE ADD COLUMN` fails loud with `Error::UnsupportedType`
    /// — the engine never silently stores an unsupported type as `TEXT`.
    /// `true` restores the legacy lenient coercion.
    ///
    /// The toggle is per-thread (the engine is single-threaded), so set it on the
    /// thread that drives this `Database`. The two type resolvers (the `CAST`
    /// path and the DDL path) both consult it.
    #[allow(clippy::unused_self)]
    pub fn set_coerce_unknown_types(&self, coerce: bool) {
        pglet_sql::planner::set_coerce_unknown_types(coerce);
    }
}

impl Drop for Database {
    /// A database dropped with a transaction block open ends the block as a client going
    /// away ends one on the server: rolled back ([`TransactionSession::rollback`]), which
    /// saves the sequence counters the block leaves ahead of the durable ones and takes back
    /// the block's DDL that a save made durable. Unrolled, both stay as the last save left
    /// them: a reopen would hand out again the values the block's `nextval` handed out, and
    /// hold that DDL.
    ///
    /// Not while the thread panics — the catalog may be half-written, as after a kill —
    /// nor on a store with no durable state, where nothing outlives the drop, nor where
    /// the entry check refuses (`Self::enter`: a failed store, a catalog this thread
    /// cannot walk), nor after [`Self::skip_drop_rollback`]. A rollback error has nowhere
    /// to go, and the store records a refused write itself.
    fn drop(&mut self) {
        if !self.drop_rolls_back
            || std::thread::panicking()
            || self.storage.durable_version().is_none()
        {
            return;
        }
        let Some(conn_id) = self.tx.owner() else {
            return;
        };
        let Ok(_stack) = self.enter() else {
            return;
        };
        let _ = self.rollback_block(conn_id);
    }
}

/// `sql`'s raw parse: its statements as 18.6's raw parse counts them, or its lexical
/// error ([`pglet_sql::parser::raw_statements`]).
fn raw_statements(sql: &str) -> Result<Vec<&str>, Error> {
    #[cfg(test)]
    tests::count_raw_parse();
    pglet_sql::parser::raw_statements(sql)
}

/// Borrow a parsed plan as engine [`CopyParams`] when it is a single `COPY` of the
/// wanted `direction`, else a loud `Error::Execution(what)`. The shared destructure
/// behind [`Database::copy_in`] / [`Database::copy_out`] — one place builds the
/// `CopyParams`, so the two entry points can't drift.
fn copy_params_of<'p>(
    plan: &'p Plan,
    direction: CopyDirection,
    what: &str,
) -> Result<CopyParams<'p>, Error> {
    match plan {
        Plan::Copy {
            direction: dir,
            options,
        } if *dir == direction => Ok(CopyParams::from(options)),
        _ => Err(Error::Execution(what.to_string())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pglet_sql::types::Value;
    use pglet_storage::btree::{BtreeStorageWrapper, CowBtree};
    use std::cell::Cell;

    thread_local! {
        /// This thread's whole-text lexes through the facade's raw parse.
        static RAW_PARSES: Cell<usize> = const { Cell::new(0) };
        /// This thread's catalog encodes through `transaction::catalog_bytes`.
        static CATALOG_ENCODES: Cell<usize> = const { Cell::new(0) };
    }

    pub fn count_raw_parse() {
        RAW_PARSES.with(|n| n.set(n.get() + 1));
    }

    pub fn count_catalog_encode() {
        CATALOG_ENCODES.with(|n| n.set(n.get() + 1));
    }

    fn one(results: Vec<QueryResult>) -> QueryResult {
        assert_eq!(results.len(), 1);
        results.into_iter().next().unwrap()
    }

    /// A connection's settings are held only while they differ from a fresh
    /// connection's: a host minting a connection id per statement holds no entry per id,
    /// while a connection that moved a setting, and one whose block is open, keep theirs.
    /// Red with an entry made for every connection that ran a statement and never
    /// dropped: one per id.
    #[test]
    fn connection_settings_are_held_only_while_they_differ_from_the_template() {
        let mut db = Database::with_btree();
        for conn_id in 100..1100 {
            db.execute_as("SELECT 1", conn_id).unwrap();
        }
        assert!(db.settings.by_connection.len() <= 1);
        db.execute_as("SET lock_timeout = '3s'", 1).unwrap();
        db.execute_as("BEGIN", 2).unwrap();
        for conn_id in 100..1100 {
            db.execute_as("SELECT 1", conn_id).unwrap();
        }
        let held: Vec<i32> = db
            .settings
            .by_connection
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert!(
            held.contains(&1) && held.contains(&2) && held.len() <= 3,
            "{held:?}"
        );
        db.execute_as("ROLLBACK", 2).unwrap();
        db.execute_as("RESET lock_timeout", 1).unwrap();
        db.execute_as("SELECT 1", 3).unwrap();
        let held: Vec<i32> = db
            .settings
            .by_connection
            .iter()
            .map(|(id, _)| *id)
            .collect();
        assert_eq!(held, vec![3]);
    }

    #[test]
    fn create_insert_select_roundtrip() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE users (id INT, name TEXT)")
            .unwrap();
        db.execute("INSERT INTO users VALUES (1, 'Alice')").unwrap();
        db.execute("INSERT INTO users VALUES (2, 'Bob')").unwrap();

        let result = one(db.execute("SELECT * FROM users ORDER BY id").unwrap());
        assert_eq!(result.rows.len(), 2);
        assert_eq!(result.rows[0][0], Value::Int4(1));
        assert_eq!(result.rows[0][1], Value::Text("Alice".into()));
    }

    #[test]
    fn explicit_transaction_commit() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT)").unwrap();

        db.begin(7).unwrap();
        db.execute_as("INSERT INTO t VALUES (10)", 7).unwrap();
        // Read-your-own-writes within the transaction.
        let in_tx = one(db.execute_as("SELECT * FROM t", 7).unwrap());
        assert_eq!(in_tx.rows.len(), 1);
        db.commit(7).unwrap();

        let after = one(db.execute_as("SELECT * FROM t", 7).unwrap());
        assert_eq!(after.rows.len(), 1);
        assert_eq!(after.rows[0][0], Value::Int4(10));
    }

    #[test]
    fn transaction_rollback_discards_writes() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();

        db.begin(1).unwrap();
        db.execute_as("INSERT INTO t VALUES (2)", 1).unwrap();
        db.rollback(1).unwrap();

        let rows = one(db.execute_as("SELECT * FROM t", 1).unwrap()).rows;
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0][0], Value::Int4(1));
    }

    /// Every row of `SELECT id FROM t` after a statement fails inside a block.
    fn ids_of(db: &mut Database, sql: &str) -> Vec<i32> {
        one(db.execute(sql).unwrap())
            .rows
            .iter()
            .map(|r| match r[0] {
                Value::Int4(n) => n,
                ref other => panic!("expected Int4, got {other:?}"),
            })
            .collect()
    }

    /// A failed statement aborts the block, and the `COMMIT` after it publishes
    /// nothing.
    ///
    /// The headline script, measured identically on 18.6 and the native server:
    /// `CREATE TABLE t (id INT PRIMARY KEY); BEGIN; INSERT 1; INSERT 2; INSERT 1;
    /// SELECT 1; INSERT 3; COMMIT; SELECT id FROM t` → `23505`, then `25P02` for
    /// everything after it, then command tag `ROLLBACK`, then **no rows**. The embed
    /// facade used to run the two statements after the failure normally and commit
    /// `1, 2, 3`; so did the wasm bindings, which drive this same code — one npm
    /// package committing different data depending on which leg you used.
    ///
    /// **The differential fixture harness is structurally blind to this class**: every
    /// fixture runs inside `BEGIN … ROLLBACK`, which restores the snapshot, so the
    /// corpus only ever measures the `ROLLBACK` column. That is a property of the
    /// instrument, not evidence that the engine was right — hence a unit test, and
    /// hence one on `with_btree`: `MemoryStorage`'s backend rollback is a no-op, so a
    /// transaction guard written on it can pass while discarding nothing.
    #[test]
    fn a_failed_statement_aborts_the_block_and_the_commit_publishes_nothing() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT PRIMARY KEY)").unwrap();

        db.execute("BEGIN").unwrap();
        assert_eq!(db.transaction_status(), TransactionStatus::InTransaction);
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        db.execute("INSERT INTO t VALUES (2)").unwrap();

        let dup = db.execute("INSERT INTO t VALUES (1)").unwrap_err();
        assert_eq!(dup.sqlstate(), "23505");
        assert_eq!(db.transaction_status(), TransactionStatus::Failed);

        // Everything after the failure is `25P02` until the block ends — including a
        // read, and including a write that would otherwise stage a row.
        for sql in ["SELECT 1", "INSERT INTO t VALUES (3)"] {
            assert_eq!(db.execute(sql).unwrap_err().sqlstate(), "25P02", "{sql:?}");
        }

        // The `COMMIT` is Postgres's implicit rollback: tag `ROLLBACK`, no error.
        let end = one(db.execute("COMMIT").unwrap());
        assert_eq!(end.command_tag, "ROLLBACK");
        assert_eq!(db.transaction_status(), TransactionStatus::Idle);

        assert!(
            ids_of(&mut db, "SELECT id FROM t").is_empty(),
            "the aborted block must publish nothing",
        );
    }

    /// An aborted block admits **only** `ROLLBACK TO`, and a successful one clears
    /// the abort so the block carries on and commits.
    ///
    /// Measured on 18.6 and reproduced statement-for-statement here (the whole script
    /// was diffed against the oracle on the native server and came back
    /// byte-identical). It is a unit test rather than a fixture for the reason the
    /// test above gives, plus a second one specific to this shape: the differential
    /// harness reports a `@setup` **error** as the fixture's entire result, so a
    /// fixture whose script deliberately fails mid-setup asserts the setup's SQLSTATE
    /// and never reaches its own `@test` — such a fixture scores green and asserts
    /// nothing.
    ///
    /// The discriminator against "`SAVEPOINT` is quietly admitted" is the *tail*, not
    /// the SQLSTATE: had the aborted block accepted `SAVEPOINT s2`, the
    /// `ROLLBACK TO s2` after it would have succeeded, cleared the abort, and let the
    /// block commit.
    #[test]
    fn an_aborted_block_admits_only_rollback_to_and_recovers_through_it() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT PRIMARY KEY)").unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        db.execute("SAVEPOINT s").unwrap();
        db.execute("INSERT INTO t VALUES (2)").unwrap();

        let dup = db.execute("INSERT INTO t VALUES (1)").unwrap_err();
        assert_eq!(dup.sqlstate(), "23505");
        assert_eq!(db.transaction_status(), TransactionStatus::Failed);

        // `SAVEPOINT` and `RELEASE` are ordinary statements to the gate — neither is
        // an exit statement — so both are `25P02`, `RELEASE` even though it names a
        // savepoint the block really holds.
        for sql in ["SAVEPOINT s2", "RELEASE s", "SELECT 1"] {
            assert_eq!(db.execute(sql).unwrap_err().sqlstate(), "25P02", "{sql:?}");
        }

        // `ROLLBACK TO` is admitted, and succeeding clears the abort.
        let recovered = one(db.execute("ROLLBACK TO s").unwrap());
        assert_eq!(recovered.command_tag, "ROLLBACK");
        assert_eq!(db.transaction_status(), TransactionStatus::InTransaction);

        db.execute("INSERT INTO t VALUES (3)").unwrap();
        let end = one(db.execute("COMMIT").unwrap());
        assert_eq!(end.command_tag, "COMMIT");
        assert_eq!(ids_of(&mut db, "SELECT id FROM t ORDER BY id"), vec![1, 3]);
    }

    /// A `ROLLBACK TO` naming no live savepoint is `3B001` **and aborts the block**.
    ///
    /// Two independent claims: the SQLSTATE is the savepoint machinery's, while the
    /// abort is the ordinary rule that a raising statement aborts its block — which a
    /// lifecycle applied *after* a successful executor run could easily have skipped,
    /// since nothing the executor ran failed. Measured together on 18.6.
    #[test]
    fn a_rollback_to_a_missing_savepoint_is_3b001_and_aborts_the_block() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT PRIMARY KEY)").unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        assert_eq!(
            db.execute("ROLLBACK TO nope").unwrap_err().sqlstate(),
            "3B001"
        );
        assert_eq!(db.transaction_status(), TransactionStatus::Failed);
        assert_eq!(db.execute("SELECT 1").unwrap_err().sqlstate(), "25P02");

        let end = one(db.execute("COMMIT").unwrap());
        assert_eq!(end.command_tag, "ROLLBACK");
        assert!(ids_of(&mut db, "SELECT id FROM t").is_empty());
    }

    /// An **unmatched** `DO` error propagates, and the body it wrote is gone.
    ///
    /// The differential fixture beside this one (`ddl/do_unmatched_condition_
    /// propagates_23505.sql`) compares the SQLSTATE and can see nothing else: an
    /// `@expect-error` fixture's statement fails, so its rows are unassertable. What
    /// is asserted here is the other half — the row the body wrote before raising is
    /// discarded, by the statement's own auto-commit rollback rather than by the
    /// block's subtransaction, which on this path is released rather than rolled
    /// back.
    ///
    /// Read as a guard on the *release*, not on the rollback: the savepoint the block
    /// pushed must come back off the backend's stack on every path out, or the next
    /// session `ROLLBACK TO` restores a level that is one too deep.
    #[test]
    fn an_unmatched_do_error_leaves_no_rows_behind() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT PRIMARY KEY)").unwrap();

        let err = db
            .execute(
                "DO $$ BEGIN \
                   INSERT INTO t VALUES (1); \
                   INSERT INTO t VALUES (1); \
                 EXCEPTION WHEN duplicate_table THEN NULL; END $$",
            )
            .unwrap_err();
        assert_eq!(err.sqlstate(), "23505");
        assert!(ids_of(&mut db, "SELECT id FROM t").is_empty());

        // The backend's savepoint stack is balanced, so an ordinary block still
        // works afterwards — the observable that a leaked level would break.
        db.execute("BEGIN").unwrap();
        db.execute("INSERT INTO t VALUES (2)").unwrap();
        db.execute("SAVEPOINT s").unwrap();
        db.execute("INSERT INTO t VALUES (3)").unwrap();
        db.execute("ROLLBACK TO s").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(ids_of(&mut db, "SELECT id FROM t ORDER BY id"), vec![2]);
    }

    /// Savepoints work on **plain in-memory storage** too.
    ///
    /// This backend buffers no pending writes, so it stages a savepoint the only way
    /// it can — a whole-store clone, the same channel its `BEGIN` snapshot uses —
    /// while the B+tree clones a transaction's pending buffer. Every other savepoint
    /// assertion in this repo runs on the B+tree, so without this one the memory
    /// backend could refuse or silently no-op them and nothing would notice.
    ///
    /// Refusing here (`0A000`) was considered and rejected: refusing would have made the same SQL answer differently per
    /// backend, and `DO … EXCEPTION` — which needs the same primitive — would then
    /// have been correct on one backend and not the other.
    #[test]
    fn savepoints_work_on_plain_memory_storage() {
        let mut db = Database::in_memory();
        db.execute("CREATE TABLE t (id INT PRIMARY KEY)").unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        db.execute("SAVEPOINT s").unwrap();
        db.execute("INSERT INTO t VALUES (2)").unwrap();
        db.execute("ROLLBACK TO s").unwrap();
        db.execute("INSERT INTO t VALUES (3)").unwrap();
        db.execute("COMMIT").unwrap();

        assert_eq!(ids_of(&mut db, "SELECT id FROM t ORDER BY id"), vec![1, 3]);

        // A **second** block, and it is the half that failed: this backend keeps no
        // per-transaction state for `commit` to remove, so the first block's savepoint
        // snapshot stayed on its stack and the `SAVEPOINT` below tripped the session's
        // lockstep check — `0 session frames vs 1 storage levels`. The `ROLLBACK` path
        // hid it (it restores a `BEGIN` clone whose stack is empty by construction), so
        // a test that ended at the `COMMIT` above, or rolled back instead, saw nothing.
        db.execute("BEGIN").unwrap();
        db.execute("SAVEPOINT s2").unwrap();
        db.execute("INSERT INTO t VALUES (4)").unwrap();
        db.execute("ROLLBACK TO s2").unwrap();
        db.execute("COMMIT").unwrap();
        assert_eq!(ids_of(&mut db, "SELECT id FROM t ORDER BY id"), vec![1, 3]);
    }

    /// The other polarity: a block that never fails still commits.
    ///
    /// Without it, a session that aborted too eagerly — on a notice, on a `SELECT`
    /// returning no rows, on any non-error — would satisfy the test above and lose
    /// every write in the engine.
    #[test]
    fn a_healthy_block_still_commits_everything_it_wrote() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT PRIMARY KEY)").unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        db.execute("SELECT * FROM t WHERE id = 99").unwrap();
        db.execute("INSERT INTO t VALUES (2)").unwrap();
        assert_eq!(db.transaction_status(), TransactionStatus::InTransaction);

        let end = one(db.execute("COMMIT").unwrap());
        assert_eq!(end.command_tag, "COMMIT");
        assert_eq!(ids_of(&mut db, "SELECT id FROM t ORDER BY id"), vec![1, 2]);
    }

    /// A block on **plain in-memory storage** rolls back too.
    ///
    /// `MemoryStorage` buffers no pending writes of its own, so the session carries a
    /// whole clone of it in the frame and restores that. This is what makes the
    /// backend the axis rather than persistence: before the session, `Database::
    /// rollback` on this backend discarded nothing at all and the write stood.
    #[test]
    fn a_rolled_back_block_on_memory_storage_discards_its_writes() {
        let mut db = Database::in_memory();
        db.execute("CREATE TABLE t (id INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();

        db.execute("BEGIN").unwrap();
        db.execute("INSERT INTO t VALUES (2)").unwrap();
        db.execute("CREATE TABLE gone (a INT)").unwrap();
        assert_eq!(ids_of(&mut db, "SELECT id FROM t ORDER BY id"), vec![1, 2]);

        db.execute("ROLLBACK").unwrap();
        assert_eq!(ids_of(&mut db, "SELECT id FROM t"), vec![1]);
        // The catalog goes back with it, and the name is free rather than a phantom.
        db.execute("CREATE TABLE gone (a INT)")
            .expect("the rolled-back name must be free again, not a 42P07 phantom");
    }

    /// The **programmatic** commit of an aborted block publishes nothing either.
    ///
    /// `Database::commit` is a second entry point to the same block, and the SQL
    /// `COMMIT` is converted to a rollback by the gate before it ever reaches the
    /// session. Without the session's own conversion the two disagreed: the SQL route
    /// discarded the failed statement's staged effects and this one published them —
    /// a half-applied `UPDATE` among the shapes, reachable from the embed API and from
    /// the npm adapter's `transaction()` alike.
    #[test]
    fn a_programmatic_commit_of_an_aborted_block_publishes_nothing() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (a INT, CHECK (a < 100))")
            .unwrap();
        db.execute("INSERT INTO t VALUES (1), (2), (3)").unwrap();

        db.begin(1).unwrap();
        // A multi-row UPDATE that violates the CHECK partway through: on this backend
        // the rows it already rewrote stay staged when it raises.
        db.execute_as("UPDATE t SET a = a * 50", 1).unwrap_err();
        assert_eq!(db.transaction_status(), TransactionStatus::Failed);

        db.commit(1).unwrap();
        assert_eq!(db.transaction_status(), TransactionStatus::Idle);
        assert_eq!(
            ids_of(&mut db, "SELECT a FROM t ORDER BY a"),
            vec![1, 2, 3],
            "a half-applied update must not survive the commit",
        );
    }

    /// A parameter-binding failure fails an open block, as an error at the native
    /// server's `Bind` does and as 18.6 fails it: the next statement is `25P02`, and the
    /// commit publishes nothing the block wrote. With no block open it changes nothing.
    #[test]
    fn a_binding_failure_fails_the_open_block() {
        let failing_bind =
            |_: &[Option<TypeOid>]| -> Result<Vec<Option<Value>>, &'static str> { Err("refused") };
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (a INT)").unwrap();

        db.begin(1).unwrap();
        db.execute_as("INSERT INTO t VALUES (1)", 1).unwrap();
        let err = db
            .execute_bound("INSERT INTO t VALUES ($1)", 1, failing_bind)
            .unwrap_err();
        assert!(matches!(err, BoundExecError::Bind("refused")), "{err:?}");
        assert_eq!(db.transaction_status(), TransactionStatus::Failed);
        let refused = db.execute_as("INSERT INTO t VALUES (2)", 1).unwrap_err();
        assert_eq!(refused.sqlstate(), "25P02");
        db.commit(1).unwrap();
        assert_eq!(ids_of(&mut db, "SELECT a FROM t"), Vec::<i32>::new());

        // The control: no block open.
        db.execute_bound("INSERT INTO t VALUES ($1)", 1, failing_bind)
            .unwrap_err();
        assert_eq!(db.transaction_status(), TransactionStatus::Idle);
        db.execute("INSERT INTO t VALUES (3)").unwrap();
        assert_eq!(ids_of(&mut db, "SELECT a FROM t"), vec![3]);
    }

    /// A statement on a second logical connection is not part of the first's block.
    ///
    /// `execute_as` routes several logical connections through one `Database`, and the
    /// session belongs to the `conn_id` that opened it. Without that, connection 2's
    /// autocommit `CREATE TABLE` would accumulate into connection 1's frame and
    /// connection 1's `ROLLBACK` would clobber it.
    #[test]
    fn another_connections_statement_neither_joins_nor_aborts_the_block() {
        let mut db = Database::with_btree();
        db.execute_as("CREATE TABLE base (id INT)", 2).unwrap();

        db.begin(1).unwrap();
        db.execute_as("SELECT count(*) FROM base", 1).unwrap();

        // Connection 1's block has only read, so connection 2's write runs; it commits
        // before connection 1's first write, which re-bases the block on it.
        db.execute_as("CREATE TABLE concurrent_t (x INT)", 2)
            .unwrap();
        db.execute_as("INSERT INTO base VALUES (1)", 1).unwrap();

        // Now the block holds the writer: connection 2's write is refused `55P03` — this
        // host cannot make it wait — and neither that nor a failing read of its own
        // aborts connection 1's block.
        let refused = db
            .execute_as("INSERT INTO concurrent_t VALUES (7)", 2)
            .unwrap_err();
        assert_eq!(refused.sqlstate(), "55P03", "{refused}");
        db.execute_as("SELECT * FROM no_such_table", 2).unwrap_err();
        assert_eq!(db.transaction_status(), TransactionStatus::InTransaction);
        db.execute_as("INSERT INTO base VALUES (9)", 1).unwrap();

        db.rollback(1).unwrap();
        // Connection 2's committed DDL survived connection 1's catalog restore, and its
        // refused row never landed.
        db.execute_as("INSERT INTO concurrent_t VALUES (42)", 2)
            .unwrap();
        assert!(ids_of(&mut db, "SELECT id FROM base").is_empty());
        assert_eq!(ids_of(&mut db, "SELECT x FROM concurrent_t"), vec![42]);
    }

    // ========================================================================
    // Index-accelerated point reads: read-your-own-writes correctness
    //
    // Index entries are maintained at COMMIT, so a naive committed-index probe
    // would disagree with the read-your-own-writes scan path inside an open
    // transaction. These tests assert an `IndexScan` returns EXACTLY the rows a
    // `Filter(Scan)` would, including uncommitted INSERT/UPDATE/DELETE. They fail
    // under the naive probe (which reads only the committed index) and pass with
    // the transaction-aware `index_lookup_in_tx` (scan-of-pending filtered).
    // ========================================================================

    /// Assert that `SELECT … WHERE k = <lit>` over a schema with an index on `k`
    /// plans to an `IndexScan` (proves the autocommit fast-probe path is taken;
    /// the same plan shape runs inside a transaction).
    ///
    /// Acceleration-only: the stripped build plans `Filter(Scan)` here. The *result*
    /// assertions in the tests below are deliberately ungated — they must hold on
    /// both profiles, which is the invariant this whole section exists to check.
    #[cfg(feature = "index_acceleration")]
    fn assert_select_k_plans_index_scan(db: &Database) {
        use pglet_sql::parser::parse;
        use pglet_sql::planner::Plan;
        let mut plans = parse("SELECT id FROM t WHERE k = 10", db.schema()).unwrap();
        let planned = plans.remove(0);
        let Plan::Project { input, .. } = planned else {
            panic!("expected a Project");
        };
        assert!(
            matches!(*input, Plan::IndexScan { ref key_column, .. } if key_column == "k"),
            "WHERE k = 10 must plan as an IndexScan, got {input:?}"
        );
    }

    /// Sorted `id` column values from a `SELECT id …` result.
    fn ids(result: &QueryResult) -> Vec<i32> {
        let mut v: Vec<i32> = result
            .rows
            .iter()
            .map(|r| match r[0] {
                Value::Int4(n) => n,
                ref other => panic!("expected Int4 id, got {other:?}"),
            })
            .collect();
        v.sort_unstable();
        v
    }

    fn index_db() -> Database {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT, k INT)").unwrap();
        // Seed committed rows: id 1,3 have k=10; id 2 has k=20.
        db.execute("INSERT INTO t VALUES (1, 10), (2, 20), (3, 10)")
            .unwrap();
        db.execute("CREATE INDEX ix_k ON t(k)").unwrap();
        db
    }

    #[test]
    fn autocommit_point_read_uses_fast_probe() {
        let db = index_db();
        // The plan is an IndexScan (fast committed-index probe in autocommit).
        #[cfg(feature = "index_acceleration")]
        assert_select_k_plans_index_scan(&db);

        let mut db = db;
        // Duplicate-value probe returns both committed rows.
        let r = one(db.execute("SELECT id FROM t WHERE k = 10").unwrap());
        assert_eq!(ids(&r), vec![1, 3]);
        // Single match.
        let r = one(db.execute("SELECT id FROM t WHERE k = 20").unwrap());
        assert_eq!(ids(&r), vec![2]);
        // Zero match.
        let r = one(db.execute("SELECT id FROM t WHERE k = 99").unwrap());
        assert!(r.rows.is_empty());
    }

    #[test]
    fn index_scan_sees_uncommitted_insert() {
        let mut db = index_db();
        db.begin(1).unwrap();
        // Uncommitted INSERT of a row matching k = 10.
        db.execute_as("INSERT INTO t VALUES (4, 10)", 1).unwrap();

        // The IndexScan MUST include the uncommitted row 4 (read-your-own-writes).
        let by_index = one(db.execute_as("SELECT id FROM t WHERE k = 10", 1).unwrap());
        assert_eq!(
            ids(&by_index),
            vec![1, 3, 4],
            "uncommitted insert must be visible to the index probe"
        );

        // Cross-check: identical to a scan-path filter over the same transaction.
        let by_scan = one(db.execute_as("SELECT id, k FROM t", 1).unwrap());
        let scan_ids: Vec<i32> = {
            let mut v: Vec<i32> = by_scan
                .rows
                .iter()
                .filter(|r| r[1] == Value::Int4(10))
                .map(|r| match r[0] {
                    Value::Int4(n) => n,
                    _ => unreachable!(),
                })
                .collect();
            v.sort_unstable();
            v
        };
        assert_eq!(ids(&by_index), scan_ids, "index path must match scan path");
        db.rollback(1).unwrap();
    }

    #[test]
    fn index_scan_sees_uncommitted_update_onto_and_off_value() {
        let mut db = index_db();
        db.begin(1).unwrap();
        // Move row 2 (k=20) ONTO k=10, and row 1 (k=10) OFF to k=30, uncommitted.
        db.execute_as("UPDATE t SET k = 10 WHERE id = 2", 1)
            .unwrap();
        db.execute_as("UPDATE t SET k = 30 WHERE id = 1", 1)
            .unwrap();

        // k=10 now: row 3 (untouched) + row 2 (moved on), NOT row 1 (moved off).
        let r = one(db.execute_as("SELECT id FROM t WHERE k = 10", 1).unwrap());
        assert_eq!(
            ids(&r),
            vec![2, 3],
            "update onto/off the value must reflect"
        );

        // The row moved off appears under its new value.
        let r = one(db.execute_as("SELECT id FROM t WHERE k = 30", 1).unwrap());
        assert_eq!(ids(&r), vec![1]);
        db.rollback(1).unwrap();
    }

    #[test]
    fn index_scan_sees_uncommitted_delete() {
        let mut db = index_db();
        db.begin(1).unwrap();
        // Delete one of the two k=10 rows, uncommitted.
        db.execute_as("DELETE FROM t WHERE id = 1", 1).unwrap();

        // The deleted row must NOT appear in the index probe.
        let r = one(db.execute_as("SELECT id FROM t WHERE k = 10", 1).unwrap());
        assert_eq!(
            ids(&r),
            vec![3],
            "uncommitted delete must drop from the probe"
        );
        db.rollback(1).unwrap();
    }

    #[test]
    fn index_scan_null_key_returns_no_rows() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (id INT, k INT)").unwrap();
        db.execute("INSERT INTO t VALUES (1, 10)").unwrap();
        db.execute("CREATE INDEX ix_k ON t(k)").unwrap();
        // `k = NULL` matches nothing (SQL equality never matches NULL).
        let r = one(db.execute("SELECT id FROM t WHERE k = NULL").unwrap());
        assert!(r.rows.is_empty(), "k = NULL must return no rows");
    }

    #[test]
    fn injection_seam_over_custom_btree_store() {
        // A future bench crate would substitute its own `PageStore` here; this
        // exercises the seam with the in-memory `CowPageStore` default, building a
        // `Database` over a caller-constructed `BtreeStorageWrapper` without any
        // server involvement.
        let btree = CowBtree::new();
        // Recover over the empty injected tree (no system entries yet → empty
        // wrapper); exercises the seam without server involvement.
        let (wrapper, _schema_bytes) =
            BtreeStorageWrapper::recover(std::sync::Arc::new(btree), 1).unwrap();
        let mut db = Database::with_storage(StorageBackend::Btree(wrapper), Schema::new());

        db.execute("CREATE TABLE k (v INT)").unwrap();
        db.execute("INSERT INTO k VALUES (42)").unwrap();
        let rows = one(db.execute("SELECT v FROM k").unwrap()).rows;
        assert_eq!(rows, vec![vec![Value::Int4(42)]]);
    }

    // ========================================================================
    // FOREIGN KEY enforcement (child existence + ON DELETE actions).
    // ========================================================================

    /// Sorted `id` values from an `INT`-id `SELECT id …` result.
    fn int_ids(result: &QueryResult) -> Vec<i32> {
        let mut v: Vec<i32> = result
            .rows
            .iter()
            .map(|r| match r[0] {
                Value::Int4(n) => n,
                ref other => panic!("expected Int4 id, got {other:?}"),
            })
            .collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn fk_missing_parent_insert_rejected() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE parent (id INT PRIMARY KEY)")
            .unwrap();
        db.execute("CREATE TABLE child (id INT PRIMARY KEY, pid INT REFERENCES parent(id))")
            .unwrap();
        // No parent row 1 exists → reject.
        let err = db
            .execute("INSERT INTO child VALUES (1, 1)")
            .expect_err("missing parent must be rejected");
        assert_eq!(err.sqlstate(), "23503");

        // Insert the parent, then the child is allowed.
        db.execute("INSERT INTO parent VALUES (1)").unwrap();
        db.execute("INSERT INTO child VALUES (1, 1)").unwrap();
        let rows = one(db.execute("SELECT id FROM child").unwrap()).rows;
        assert_eq!(rows.len(), 1);
    }

    #[test]
    fn fk_null_value_insert_allowed() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE parent (id INT PRIMARY KEY)")
            .unwrap();
        db.execute("CREATE TABLE child (id INT PRIMARY KEY, pid INT REFERENCES parent(id))")
            .unwrap();
        // A NULL FK value needs no parent (the FK permits NULL).
        db.execute("INSERT INTO child VALUES (1, NULL)").unwrap();
        let r = one(db.execute("SELECT id FROM child").unwrap());
        assert_eq!(int_ids(&r), vec![1]);
    }

    #[test]
    fn fk_self_referential_set_null() {
        // actor.updated_by -> actor(id) ON DELETE SET NULL (a real spine shape).
        let mut db = Database::with_btree();
        db.execute(
            "CREATE TABLE actor (id INT PRIMARY KEY, updated_by INT REFERENCES actor(id) ON DELETE SET NULL)",
        )
        .unwrap();
        db.execute("INSERT INTO actor VALUES (1, NULL)").unwrap();
        // Row 2 is updated_by row 1.
        db.execute("INSERT INTO actor VALUES (2, 1)").unwrap();

        // Delete actor 1: row 2's updated_by must become NULL (SET NULL), row 2 survives.
        db.execute("DELETE FROM actor WHERE id = 1").unwrap();
        let r = one(db.execute("SELECT id FROM actor ORDER BY id").unwrap());
        assert_eq!(int_ids(&r), vec![2], "only the deleted parent is gone");
        let updated_by = one(db
            .execute("SELECT updated_by FROM actor WHERE id = 2")
            .unwrap());
        assert_eq!(
            updated_by.rows[0][0],
            Value::Null,
            "the child's FK column is nulled by ON DELETE SET NULL"
        );
    }

    #[test]
    fn fk_cascade_is_atomic_multi_level() {
        // grandparent <- parent <- child, both CASCADE. Deleting the grandparent
        // removes the whole chain, and it lands as one commit (atomic): the rows
        // are gone on the next autocommit read.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE gp (id INT PRIMARY KEY)").unwrap();
        db.execute(
            "CREATE TABLE p (id INT PRIMARY KEY, gp_id INT REFERENCES gp(id) ON DELETE CASCADE)",
        )
        .unwrap();
        db.execute(
            "CREATE TABLE c (id INT PRIMARY KEY, p_id INT REFERENCES p(id) ON DELETE CASCADE)",
        )
        .unwrap();
        db.execute("INSERT INTO gp VALUES (1), (2)").unwrap();
        db.execute("INSERT INTO p VALUES (10, 1), (11, 1), (20, 2)")
            .unwrap();
        db.execute("INSERT INTO c VALUES (100, 10), (101, 11), (200, 20)")
            .unwrap();

        // Delete grandparent 1: p {10,11} and c {100,101} cascade away; gp 2's
        // subtree (p 20, c 200) survives.
        let res = one(db.execute("DELETE FROM gp WHERE id = 1").unwrap());
        // The command tag counts only the directly-deleted row (Postgres semantics).
        assert_eq!(res.command_tag, "DELETE 1");

        let gp = one(db.execute("SELECT id FROM gp ORDER BY id").unwrap());
        assert_eq!(int_ids(&gp), vec![2]);
        let p = one(db.execute("SELECT id FROM p ORDER BY id").unwrap());
        assert_eq!(int_ids(&p), vec![20], "p rows under gp 1 cascade-deleted");
        let c = one(db.execute("SELECT id FROM c ORDER BY id").unwrap());
        assert_eq!(int_ids(&c), vec![200], "c rows under gp 1 cascade-deleted");
    }

    #[test]
    fn fk_restrict_blocks_parent_delete() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE parent (id INT PRIMARY KEY)")
            .unwrap();
        // Default (no ON DELETE) is NO ACTION → restrict.
        db.execute("CREATE TABLE child (id INT PRIMARY KEY, pid INT REFERENCES parent(id))")
            .unwrap();
        db.execute("INSERT INTO parent VALUES (1)").unwrap();
        db.execute("INSERT INTO child VALUES (1, 1)").unwrap();

        let err = db
            .execute("DELETE FROM parent WHERE id = 1")
            .expect_err("a referenced parent must not be deletable under NO ACTION");
        assert_eq!(err.sqlstate(), "23503");
        // The parent (and child) are untouched after the aborted delete.
        let p = one(db.execute("SELECT id FROM parent").unwrap());
        assert_eq!(p.rows.len(), 1);
    }

    #[test]
    fn fk_cascade_cycle_guard_terminates() {
        // A self-referential CASCADE forms the tightest cascade cycle: a row that
        // references itself. Deleting it cascades to "children referencing id = 1"
        // — which is the row itself, already in the visited set — so the guard
        // stops the loop instead of re-deleting forever. Postgres also self-
        // references via SET NULL only (never CASCADE), so this can't arise from
        // the spine, but the engine must still terminate.
        let mut db = Database::with_btree();
        db.execute(
            "CREATE TABLE node (id INT PRIMARY KEY, parent_id INT REFERENCES node(id) ON DELETE CASCADE)",
        )
        .unwrap();
        // node(1) references itself.
        db.execute("INSERT INTO node VALUES (1, NULL)").unwrap();
        db.execute("UPDATE node SET parent_id = 1 WHERE id = 1")
            .unwrap();
        // node(2) is a child of node(1).
        db.execute("INSERT INTO node VALUES (2, 1)").unwrap();

        // Deleting node(1) cascades to node(2) and (via the self-reference) back to
        // node(1) — which the guard skips. Must terminate, leaving the table empty.
        db.execute("DELETE FROM node WHERE id = 1").unwrap();
        let r = one(db.execute("SELECT id FROM node").unwrap());
        assert!(
            r.rows.is_empty(),
            "the self-referential CASCADE deletes the whole component and terminates"
        );
    }

    // ========================================================================
    // fork(): the per-test-fixture primitive — a CoW branch of committed state.
    //
    // A fork must be a fully isolated copy: writes on either the base or the fork
    // are invisible to the other, and the fork carries the base's schema, data,
    // indexes, and constraints. These tests drive both directions of isolation
    // plus the carried-catalog and error paths.
    // ========================================================================

    fn seeded_base() -> Database {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE users (id INT PRIMARY KEY, name TEXT NOT NULL)")
            .unwrap();
        db.execute("CREATE INDEX ix_name ON users(name)").unwrap();
        db.execute("INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob')")
            .unwrap();
        db
    }

    /// Sorted `id`s from a `SELECT id …` result over an `INT` id column.
    fn fork_ids(result: &QueryResult) -> Vec<i32> {
        let mut v: Vec<i32> = result
            .rows
            .iter()
            .map(|r| match r[0] {
                Value::Int4(n) => n,
                ref other => panic!("expected Int4 id, got {other:?}"),
            })
            .collect();
        v.sort_unstable();
        v
    }

    #[test]
    fn fork_writes_do_not_leak_into_base() {
        let base = seeded_base();
        let mut fork = base.fork().unwrap();

        // The fork starts identical to the base.
        assert_eq!(
            fork_ids(&one(fork.execute("SELECT id FROM users").unwrap())),
            vec![1, 2]
        );

        // Mutate the fork: insert, update, delete.
        fork.execute("INSERT INTO users VALUES (3, 'Carol')")
            .unwrap();
        fork.execute("UPDATE users SET name = 'Alicia' WHERE id = 1")
            .unwrap();
        fork.execute("DELETE FROM users WHERE id = 2").unwrap();

        // The fork reflects its own writes.
        assert_eq!(
            fork_ids(&one(fork.execute("SELECT id FROM users").unwrap())),
            vec![1, 3]
        );

        // The base is untouched — the whole point of the fork.
        let mut base = base;
        assert_eq!(
            fork_ids(&one(base.execute("SELECT id FROM users").unwrap())),
            vec![1, 2]
        );
        let alice = one(base.execute("SELECT name FROM users WHERE id = 1").unwrap());
        assert_eq!(alice.rows[0][0], Value::Text("Alice".into()));
    }

    #[test]
    fn base_writes_after_fork_do_not_leak_into_fork() {
        let base = seeded_base();
        let mut fork = base.fork().unwrap();

        // Mutate the base *after* forking; the fork must not observe it.
        let mut base = base;
        base.execute("INSERT INTO users VALUES (9, 'Zed')").unwrap();
        base.execute("DELETE FROM users WHERE id = 1").unwrap();

        assert_eq!(
            fork_ids(&one(base.execute("SELECT id FROM users").unwrap())),
            vec![2, 9]
        );
        // The fork still sees the committed-at-fork state.
        assert_eq!(
            fork_ids(&one(fork.execute("SELECT id FROM users").unwrap())),
            vec![1, 2]
        );
    }

    #[test]
    fn fork_carries_indexes_and_constraints() {
        let base = seeded_base();
        let mut fork = base.fork().unwrap();

        // The secondary index came across: a point read on `name` plans + probes it
        // and returns the right row (the def map, not just the tree entries, forked).
        let by_name = one(fork
            .execute("SELECT id FROM users WHERE name = 'Bob'")
            .unwrap());
        assert_eq!(fork_ids(&by_name), vec![2]);

        // PRIMARY KEY uniqueness is enforced independently on the fork.
        let dup = fork
            .execute("INSERT INTO users VALUES (1, 'Dup')")
            .expect_err("PK uniqueness must be enforced on the fork");
        assert_eq!(dup.sqlstate(), "23505");

        // NOT NULL too.
        let null_name = fork
            .execute("INSERT INTO users VALUES (5, NULL)")
            .expect_err("NOT NULL must be enforced on the fork");
        assert_eq!(null_name.sqlstate(), "23502");

        // A fresh id inserts fine (the row-id counter forked, so no id clash).
        fork.execute("INSERT INTO users VALUES (7, 'Grace')")
            .unwrap();
        assert_eq!(
            fork_ids(&one(fork.execute("SELECT id FROM users").unwrap())),
            vec![1, 2, 7]
        );
    }

    #[test]
    fn fork_of_fork_is_independent() {
        let base = seeded_base();
        let mut fork1 = base.fork().unwrap();
        fork1
            .execute("INSERT INTO users VALUES (3, 'Carol')")
            .unwrap();

        // Fork the fork: fork2 inherits fork1's committed state (incl. Carol).
        let mut fork2 = fork1.fork().unwrap();
        assert_eq!(
            fork_ids(&one(fork2.execute("SELECT id FROM users").unwrap())),
            vec![1, 2, 3]
        );

        fork2.execute("DELETE FROM users WHERE id = 3").unwrap();

        // fork2's delete doesn't touch fork1.
        assert_eq!(
            fork_ids(&one(fork1.execute("SELECT id FROM users").unwrap())),
            vec![1, 2, 3]
        );
        assert_eq!(
            fork_ids(&one(fork2.execute("SELECT id FROM users").unwrap())),
            vec![1, 2]
        );
    }

    #[test]
    fn fork_transaction_on_fork_is_isolated() {
        let base = seeded_base();
        let mut fork = base.fork().unwrap();

        // A committed transaction on the fork lands on the fork only.
        fork.begin(1).unwrap();
        fork.execute_as("INSERT INTO users VALUES (4, 'Dave')", 1)
            .unwrap();
        fork.commit(1).unwrap();
        assert_eq!(
            fork_ids(&one(fork.execute("SELECT id FROM users").unwrap())),
            vec![1, 2, 4]
        );

        let mut base = base;
        assert_eq!(
            fork_ids(&one(base.execute("SELECT id FROM users").unwrap())),
            vec![1, 2]
        );
    }

    /// The `25001` refusal, and the two silent corruptions it stands in for.
    ///
    /// Both directions were measured before the gate landed: a pending `DROP TABLE`
    /// gave the fork `42P01` over a committed table whose rows were still in the
    /// forked tree, and a pending `ADD COLUMN` gave it a 3-column descriptor over
    /// 2-value rows. Neither is observable now, so what the test can assert is the
    /// refusal — revert the gate in `fork` and both arms go green-then-wrong.
    #[test]
    fn fork_refuses_while_a_transaction_is_open() {
        // Direction 1: a pending DROP TABLE would hide a committed table.
        let mut base = seeded_base();
        base.begin(1).unwrap();
        base.execute_as("DROP TABLE users", 1).unwrap();
        let err = base.fork().expect_err("a pending DROP TABLE must not fork");
        assert_eq!(err.sqlstate(), "25001", "{err}");
        assert!(err.to_string().contains("fork()"), "{err}");

        // The refusal leaves the source's block intact and usable.
        base.execute_as("CREATE TABLE t (id INT)", 1).unwrap();
        base.rollback(1).unwrap();
        assert_eq!(
            fork_ids(&one(base.execute("SELECT id FROM users").unwrap())),
            vec![1, 2],
            "the rolled-back block must leave the committed rows"
        );

        // Direction 2: a pending ADD COLUMN would widen the descriptor past the rows.
        let mut base2 = seeded_base();
        base2.begin(1).unwrap();
        base2
            .execute_as("ALTER TABLE users ADD COLUMN age INT", 1)
            .unwrap();
        assert_eq!(
            base2
                .fork()
                .expect_err("a pending ADD COLUMN must not fork")
                .sqlstate(),
            "25001"
        );
    }

    /// An **aborted** block is open too, which `status() != Idle` covers and an
    /// `== InTransaction` test would not — and it is the worse case, since the
    /// failed statement's catalog writes are exactly what no rollback puts back.
    #[test]
    fn fork_refuses_while_the_block_is_aborted() {
        let mut base = seeded_base();
        base.begin(1).unwrap();
        base.execute_as("SELECT nosuchcolumn FROM users", 1)
            .unwrap_err();
        assert_eq!(base.transaction_status(), TransactionStatus::Failed);
        assert_eq!(
            base.fork()
                .expect_err("an aborted block must not fork")
                .sqlstate(),
            "25001"
        );
    }

    /// The control: the gate is on an *open* block, not on having used one. Both
    /// exits release it, and the fork then carries what that exit decided.
    #[test]
    fn fork_succeeds_after_the_block_ends() {
        let mut base = seeded_base();
        base.begin(1).unwrap();
        base.execute_as("INSERT INTO users VALUES (3, 'Cleo')", 1)
            .unwrap();
        base.commit(1).unwrap();
        let mut committed = base.fork().expect("a committed block leaves nothing open");
        assert_eq!(
            fork_ids(&one(committed.execute("SELECT id FROM users").unwrap())),
            vec![1, 2, 3]
        );

        base.begin(1).unwrap();
        base.execute_as("INSERT INTO users VALUES (4, 'Dave')", 1)
            .unwrap();
        base.rollback(1).unwrap();
        let mut rolled_back = base
            .fork()
            .expect("a rolled-back block leaves nothing open");
        assert_eq!(
            fork_ids(&one(rolled_back.execute("SELECT id FROM users").unwrap())),
            vec![1, 2, 3]
        );
    }

    /// `DEALLOCATE` on a host that holds no named prepared statements.
    ///
    /// The embed facade and the wasm bindings bind parameter values directly and
    /// mint no statement handle, so their namespace is empty — and an empty one is
    /// not a degenerate case to code around: 18.6 answers exactly this for a session
    /// that has prepared nothing. Without it the wasm leg would be `42601` where the
    /// server leg succeeds, which is a host divergence on the statement a pool sends
    /// on every connection release.
    #[test]
    fn deallocate_over_an_empty_namespace_matches_postgres() {
        let mut db = Database::with_btree();
        for sql in ["DEALLOCATE ALL", "DEALLOCATE PREPARE ALL"] {
            assert_eq!(one(db.execute(sql).unwrap()).command_tag, "DEALLOCATE ALL");
        }
        let err = db.execute("DEALLOCATE nosuch").unwrap_err();
        assert_eq!(err.sqlstate(), "26000", "{err}");
        assert_eq!(db.execute("DEALLOCATE;").unwrap_err().sqlstate(), "42601");
        // A quoted leading word is a name, not this statement — so it falls through
        // to the parser, which has nothing that starts with an identifier.
        assert_eq!(
            db.execute(r#""deallocate" x"#).unwrap_err().sqlstate(),
            "42601"
        );
    }

    /// The interception sits behind the aborted-block gate on this host too — the
    /// executed statement is not the place that decides, and a gate written beside
    /// the interception rather than ahead of it would let a dead block run it.
    #[test]
    fn deallocate_in_an_aborted_block_is_refused() {
        let mut db = Database::with_btree();
        db.begin(1).unwrap();
        db.execute_as("SELECT nosuchthing", 1).unwrap_err();
        assert_eq!(
            db.execute_as("DEALLOCATE ALL", 1).unwrap_err().sqlstate(),
            "25P02"
        );
    }

    #[test]
    fn fork_rejects_non_cow_backends() {
        // Plain in-memory storage has no CoW tree to branch.
        let mem = Database::in_memory();
        assert!(mem.fork().is_err(), "in_memory() has no CoW fork");
    }

    // ========================================================================
    // Sequence functions as expressions — nextval / currval / setval.
    // ========================================================================

    /// The single int8 value of a `SELECT nextval/currval/setval(...)` result.
    fn seq_val(result: &QueryResult) -> i64 {
        assert_eq!(result.rows.len(), 1);
        assert_eq!(result.rows[0].len(), 1);
        match result.rows[0][0] {
            Value::Int8(v) => v,
            ref other => panic!("expected int8, got {other:?}"),
        }
    }

    #[test]
    fn nextval_advances_from_one() {
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        assert_eq!(seq_val(&one(db.execute("SELECT nextval('s')").unwrap())), 1);
        assert_eq!(seq_val(&one(db.execute("SELECT nextval('s')").unwrap())), 2);
    }

    #[test]
    fn setval_two_arg_then_nextval() {
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        // setval echoes its value; the next nextval returns n + 1.
        assert_eq!(
            seq_val(&one(db.execute("SELECT setval('s', 10)").unwrap())),
            10
        );
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('s')").unwrap())),
            11
        );
    }

    #[test]
    fn setval_is_called_false_then_nextval() {
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        // is_called = false: the next nextval returns exactly n (not n + 1).
        assert_eq!(
            seq_val(&one(db.execute("SELECT setval('s', 10, false)").unwrap())),
            10
        );
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('s')").unwrap())),
            10
        );
    }

    #[test]
    fn currval_reports_last_nextval() {
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        db.execute("SELECT nextval('s')").unwrap();
        db.execute("SELECT nextval('s')").unwrap();
        assert_eq!(seq_val(&one(db.execute("SELECT currval('s')").unwrap())), 2);
        // setval with is_called = true also defines currval.
        db.execute("SELECT setval('s', 42)").unwrap();
        assert_eq!(
            seq_val(&one(db.execute("SELECT currval('s')").unwrap())),
            42
        );
    }

    #[test]
    fn currval_without_nextval_fails_55000() {
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        let err = db.execute("SELECT currval('s')").unwrap_err();
        assert_eq!(err.sqlstate(), "55000");
    }

    #[test]
    fn nextval_unknown_sequence_fails_42p01() {
        let mut db = Database::with_btree();
        let err = db.execute("SELECT nextval('nope')").unwrap_err();
        assert_eq!(err.sqlstate(), "42P01");
    }

    #[test]
    fn nextval_schema_qualified_name_resolution() {
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        // `public.s` reduces to the `s` slot (like CREATE SEQUENCE / DEFAULT nextval).
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('public.s')").unwrap())),
            1
        );
        // A non-`public` qualifier fails loud (42P01) rather than silently advancing
        // the bare `s` slot — the correctness fix.
        let err = db.execute("SELECT nextval('other.s')").unwrap_err();
        assert_eq!(err.sqlstate(), "42P01");
    }

    #[test]
    fn setval_unknown_sequence_fails_42p01() {
        let mut db = Database::with_btree();
        let err = db.execute("SELECT setval('nope', 1)").unwrap_err();
        assert_eq!(err.sqlstate(), "42P01");
    }

    #[test]
    fn nextval_in_insert_values_advances() {
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        db.execute("CREATE TABLE t (id BIGINT)").unwrap();
        db.execute("INSERT INTO t (id) VALUES (nextval('s'))")
            .unwrap();
        db.execute("INSERT INTO t (id) VALUES (nextval('s'))")
            .unwrap();
        let r = one(db.execute("SELECT id FROM t ORDER BY id").unwrap());
        assert_eq!(r.rows.len(), 2);
        assert_eq!(r.rows[0][0], Value::Int8(1));
        assert_eq!(r.rows[1][0], Value::Int8(2));
        // The explicit nextval also defined currval this session.
        assert_eq!(seq_val(&one(db.execute("SELECT currval('s')").unwrap())), 2);
    }

    #[test]
    fn nested_sequence_fn_fails_loud() {
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        // A sequence function in a nested position (a scalar subquery) is not a
        // top-level projection — pglet is stricter than Postgres here and fails loud.
        assert!(db.execute("SELECT (SELECT nextval('s'))").is_err());
        // ... and in an arithmetic operand.
        assert!(db.execute("SELECT nextval('s') + 1").is_err());
    }

    #[test]
    fn a_failed_create_table_leaves_no_phantom() {
        // The autocommit path rolls back **storage** on an error and leaves
        // `self.schema` mutated in place, so anything `execute_create_table` can raise
        // after `schema.create_table` is a *durable* half-created table — the next
        // successful write serializes the catalog with it. The differential fixtures
        // cannot see this class at all: the harness wraps every fixture in
        // `BEGIN … ROLLBACK`, which restores the catalog snapshot.
        //
        // A `PRIMARY KEY` naming a column the table lacks was the last such error, and
        // it left the name unusable: reads answered 42P01 (storage had been rolled
        // back) while `CREATE TABLE` answered 42P07 forever.
        let mut db = Database::with_btree();
        let err = db
            .execute("CREATE TABLE ph (a INT, PRIMARY KEY (b))")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42703");
        assert!(
            db.schema().get_table("ph").is_none(),
            "the failed CREATE TABLE left a phantom catalog entry"
        );

        // The behavioral half: the name is free, and the table that lands is usable.
        db.execute("CREATE TABLE ph (a INT, PRIMARY KEY (a))")
            .unwrap();
        db.execute("INSERT INTO ph VALUES (1)").unwrap();
        assert_eq!(one(db.execute("SELECT a FROM ph").unwrap()).rows.len(), 1);
    }

    #[test]
    fn a_refused_column_default_leaves_no_phantom() {
        // The DEFAULT cook is a new gate on the `CREATE TABLE` path, so it inherits
        // this path's standing requirement: everything fallible runs before
        // `schema.create_table`, because the autocommit path rolls back storage and
        // leaves `self.schema` mutated. The differential harness cannot see the class
        // (every fixture runs inside `BEGIN … ROLLBACK`), which is why the guard is
        // here.
        let mut db = Database::with_btree();
        // Each of the cook's three refusals, one per statement: the construct walk,
        // the DDL-time input function, and pglet's own unrenderable-expression gate.
        for (sql, state) in [
            (
                "CREATE TABLE ph_def (a INT, b INT DEFAULT (a + 1))",
                "0A000",
            ),
            ("CREATE TABLE ph_def (a INT, b INT DEFAULT 'x')", "22P02"),
            (
                "CREATE TABLE ph_def (a INT, b INTERVAL DEFAULT interval '1 day')",
                "0A000",
            ),
            // The render probe's **deferred-eval neighbour**, and the discriminating
            // one: its probe fails `22012` before any value exists to coerce, so a
            // cook that returns on the deferred arm never reaches the render probe at
            // all. It persisted an `Expr::Bound` (the interval literal's spelling),
            // after which `pglet_server dump` exited 1 for the *whole* database and
            // `information_schema.columns.column_default` read SQL NULL for the
            // column. The row above is green with that hole; only this one is not.
            (
                "CREATE TABLE ph_def (a INT, b TIMESTAMP DEFAULT now() + interval '1 day' * (1/0))",
                "0A000",
            ),
        ] {
            let err = db.execute(sql).unwrap_err();
            assert_eq!(err.sqlstate(), state, "{sql}");
            assert!(
                db.schema().get_table("ph_def").is_none(),
                "{sql} left a phantom catalog entry"
            );
        }
        // The name is still free and the table that lands is usable.
        db.execute("CREATE TABLE ph_def (a INT, b INT DEFAULT (1 + 1))")
            .unwrap();
        db.execute("INSERT INTO ph_def (a) VALUES (1)").unwrap();
        assert_eq!(
            one(db.execute("SELECT b FROM ph_def").unwrap()).rows[0][0],
            Value::Int4(2)
        );
    }

    #[test]
    fn a_refused_column_default_registers_no_sequence() {
        // The `CREATE TABLE` path's sequence pass runs *after* the DEFAULT cook, so a
        // refused default must not have registered the sequence its neighbour names —
        // a phantom sequence is the backend-independent half of this class (a catalog
        // residue with no storage counterpart, so even the memory double reproduces
        // it), and it burns the name forever.
        let mut db = Database::with_btree();
        let err = db
            .execute("CREATE TABLE ph_seq (a INT DEFAULT nextval('ph_s'), b INT DEFAULT 'x')")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "22P02");
        assert!(
            !db.schema().sequences.contains_key("ph_s"),
            "the refused DEFAULT left a phantom sequence"
        );
        assert!(db.schema().get_table("ph_seq").is_none());
    }

    #[test]
    fn a_refused_add_column_default_leaves_no_phantom_column() {
        // The `ADD COLUMN` twin, and the worse instance of the class: there the column
        // lands in the catalog while the widened rows roll back with storage, so the
        // table keeps two columns and one-value rows and every `SELECT *` then fails
        // at the wire, permanently, for every client. It reproduces only on the B+tree
        // backend, which is what `with_btree` is here for.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE ph_add (a INT)").unwrap();
        db.execute("INSERT INTO ph_add VALUES (1), (2)").unwrap();
        let err = db
            .execute("ALTER TABLE ph_add ADD COLUMN b INT DEFAULT 'x'")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "22P02");
        assert_eq!(
            db.schema().get_table("ph_add").unwrap().columns.len(),
            1,
            "the refused ADD COLUMN left a phantom column"
        );
        // The behavioral half: the rows still read, which is exactly what a phantom
        // column takes away.
        assert_eq!(
            one(db.execute("SELECT * FROM ph_add").unwrap()).rows.len(),
            2
        );
    }

    #[test]
    fn a_partly_built_index_set_leaves_no_phantom() {
        // The same hole one constraint later: the first PRIMARY KEY's index builds
        // fine and the *second* constraint's missing column raises — so the table, its
        // sequences, and one enforced index were all already in the catalog. Distinct
        // from the case above, where nothing in the index loop had succeeded yet.
        let mut db = Database::with_btree();
        let err = db
            .execute("CREATE TABLE ph2 (a INT, PRIMARY KEY (a), UNIQUE (nosuch))")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42703");
        assert!(db.schema().get_table("ph2").is_none());
        assert!(db.schema().get_index("ph2_pkey").is_none());
        db.execute("CREATE TABLE ph2 (a INT)").unwrap();
    }

    #[test]
    fn a_rejected_create_index_leaves_the_oid_counter_alone() {
        // `generate_object_oid` is a **catalog write**, and it used to run as
        // `execute_create_index`'s first statement — so a statement that then failed,
        // or that skipped entirely, still moved the persisted `next_oid`. No fixture
        // can see it: every fixture runs inside `BEGIN … ROLLBACK`, which restores the
        // snapshot, and the counter is invisible to SQL besides. The value is benign
        // (monotonic, never reused), but "a failed DDL leaves the catalog untouched" is
        // the invariant standing between a raised error and a durable change, and a
        // repeated idempotent `CREATE INDEX IF NOT EXISTS` — the migration idiom —
        // burned one per run.
        //
        // The name once said **catalog check**, because the allocation sat between the
        // last check and the storage build and the build could still burn one. It no
        // longer can: the number is read without advancing the counter
        // (`next_object_oid`) and written back only once the build has returned, so the
        // refusals below — the storage build's own, over a `jsonb` GIN token past the
        // page bound, and a key expression that raises on a row — belong in this test
        // rather than in its residual.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE oidc (a INT, j JSONB)").unwrap();
        db.execute(&format!(
            "INSERT INTO oidc VALUES (1, '{{\"k\":1}}'), (0, '{}')",
            long_jsonb_string()
        ))
        .unwrap();
        db.execute("CREATE INDEX oidc_ix ON oidc (a)").unwrap();
        let before = db.schema().next_oid;

        // The name conflict, its `IF NOT EXISTS` skip, and a check on either side of
        // the name in the ordering — none of them may move the counter.
        let err = db.execute("CREATE INDEX oidc_ix ON oidc (a)").unwrap_err();
        assert_eq!(err.sqlstate(), "42P07");
        db.execute("CREATE INDEX IF NOT EXISTS oidc_ix ON oidc (a)")
            .unwrap();
        db.execute("CREATE INDEX IF NOT EXISTS oidc_ix ON oidc (j)")
            .unwrap();
        let err = db
            .execute("CREATE INDEX oidc_ix2 ON oidc (nosuchcol)")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42703");
        let err = db.execute("CREATE INDEX oidc_ix3 ON oidc (j)").unwrap_err();
        assert_eq!(err.sqlstate(), "0A000");
        // Refusals no gate above reads: the storage build's own, of a GIN token
        // its page cannot hold (`long_jsonb_string`), and the expression key that
        // raises on the row holding `0`.
        let err = db
            .execute("CREATE INDEX oidc_ix4 ON oidc USING gin (j)")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "54000");
        let err = db
            .execute("CREATE INDEX oidc_ix5 ON oidc ((10 / a))")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "22012");
        assert_eq!(
            db.schema().next_oid,
            before,
            "a rejected or skipped CREATE INDEX moved the OID counter"
        );

        // And the counter still advances when the statement actually creates one.
        db.execute("CREATE INDEX oidc_ix6 ON oidc (a)").unwrap();
        assert!(db.schema().next_oid > before);
    }

    /// A `jsonb` string whose GIN token is longer than a B+tree page holds as a key.
    ///
    /// The one statement-shaped failure left **inside** the storage build: the
    /// executor's entry sizing reads a GIN index's array elements and not a `jsonb`
    /// key's tokens, so `CREATE INDEX … USING gin (j)` over a row holding this passes
    /// every gate and is refused by the back-fill itself — `54000`, pglet's own page
    /// bound (`EDGES.md` row 192; 18.6 hashes a long token and builds the index).
    /// `the_gin_token_refusal_is_the_storage_builds_own` holds that it is the storage
    /// build that refuses, which is what every test using this as its trigger needs.
    fn long_jsonb_string() -> String {
        format!("\"{}\"", "x".repeat(5000))
    }

    #[test]
    fn the_gin_token_refusal_is_the_storage_builds_own() {
        // The tests that need a failing **storage** build use `long_jsonb_string` as
        // their trigger, and they go blind the day a gate ahead of the build learns to
        // refuse it: the statement would fail as before, a step earlier, and prove
        // nothing about what a failed build leaves. So the trigger is held here, by
        // an order only the build's position gives. `execute_create_index`'s last gate
        // ahead of the build is the render probe, pglet's own `0A000` for an expression
        // the deparser cannot write back (an interval literal); a statement that trips
        // both answers the probe's `0A000` while the token is the build's to refuse,
        // and `54000` once anything ahead of the probe refuses it.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE gt (iv INTERVAL, j JSONB)")
            .unwrap();
        db.execute(&format!(
            "INSERT INTO gt VALUES ('1 day', '{}')",
            long_jsonb_string()
        ))
        .unwrap();
        let unrenderable = "WHERE (iv + interval '1 day') IS NOT NULL";
        let err = db
            .execute(&format!(
                "CREATE INDEX gt_ix ON gt USING gin (j) {unrenderable}"
            ))
            .unwrap_err();
        assert_eq!(err.sqlstate(), "0A000", "{err}");
        // Alone, each refuses for itself: the token `54000`, the predicate `0A000`.
        let err = db
            .execute("CREATE INDEX gt_ix ON gt USING gin (j)")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "54000", "{err}");
        assert!(err.to_string().contains("B+tree page"), "{err}");
        db.execute("DELETE FROM gt").unwrap();
        let err = db
            .execute(&format!(
                "CREATE INDEX gt_ix ON gt USING gin (j) {unrenderable}"
            ))
            .unwrap_err();
        assert_eq!(err.sqlstate(), "0A000", "{err}");
        db.execute("CREATE INDEX gt_ix ON gt USING gin (j)")
            .unwrap();
    }

    #[test]
    fn a_failed_create_index_build_leaves_no_phantom() {
        // The storage build is the one fallible step no DDL gate stands in front of,
        // and what reaches it from SQL is `long_jsonb_string`: a GIN token its page
        // cannot hold. (A key expression that raises — `(10 / a)` over a zero, `22012`
        // on both engines — fails the statement a step earlier, in the executor's entry
        // sizing, and is here for the catalog and the counter rather than for the
        // storage build.) What a partial predicate decides is *membership*, so the
        // predicate itself is no route: it is the admitted rows' **key** that is
        // encoded.
        // A catalog write ahead of the build was durable: the autocommit path rolls
        // back storage and leaves `schema` mutated, so `pg_indexes` listed the index,
        // its name answered `42P07` forever, and on the B+tree backend the storage def
        // registered beside it went on refusing most writes to the table.
        //
        // No fixture can see the catalog half: the harness's `BEGIN … ROLLBACK` restores
        // the snapshot, and the `DO … EXCEPTION` swallow a `@no-txn` fixture needs sheds
        // it too. The storage half is what `ddl/failed_*_index_build_*` pins.
        //
        // Both backends, because they used to answer differently — `MemoryStorage`
        // registered its def last already, so only the catalog half reproduced there,
        // and its symptom was the leaked `22000 execution error: index not found: <oid>`
        // rather than a refused write.
        //
        // The declared-constraint routes (`ADD CONSTRAINT`, `ADD PRIMARY KEY`, an
        // inline `UNIQUE`) have no leg here: a constraint's index is a B-tree over bare
        // columns, every refusal of one is raised by the executor ahead of the build,
        // and no statement was found that fails inside it.
        for (label, mut db) in [
            ("btree", Database::with_btree()),
            ("memory", Database::in_memory()),
        ] {
            db.execute("CREATE TABLE fb (a INT, f FLOAT8, j JSONB)")
                .unwrap();
            db.execute(&format!(
                "INSERT INTO fb VALUES (1, 1.5, '{}'), (0, 2.5, '{{\"k\":2}}')",
                long_jsonb_string()
            ))
            .unwrap();
            let before = db.schema().next_oid;

            // The storage build's refusal, bare and under a predicate that admits the
            // row, plus the value-driven one raised ahead of it. Each is the *whole*
            // statement, so the catalog must be untouched after every one of them.
            for (sql, state) in [
                ("CREATE INDEX fb_ix ON fb USING gin (j)", "54000"),
                (
                    "CREATE INDEX fb_ix ON fb USING gin (j) WHERE f > 0",
                    "54000",
                ),
                ("CREATE INDEX fb_ix ON fb ((10 / a))", "22012"),
            ] {
                let err = db.execute(sql).unwrap_err();
                assert_eq!(err.sqlstate(), state, "{label}: {sql}");
                assert!(
                    db.schema().get_index("fb_ix").is_none(),
                    "{label}: {sql} left a phantom index in the catalog"
                );
                assert_eq!(
                    db.schema().next_oid,
                    before,
                    "{label}: {sql} moved the OID counter"
                );
            }

            // The behavioral half, and the one the catalog assertion cannot reach: the
            // table is still writable and still readable — by a row a surviving def
            // would refuse, its token as long as the one that failed the build. On
            // B+tree a surviving storage def refused the write; on memory a unique
            // probe and an accelerated read each answered `22000 index not found`.
            db.execute(&format!(
                "INSERT INTO fb VALUES (3, 3.5, '{}')",
                long_jsonb_string()
            ))
            .unwrap();
            db.execute("UPDATE fb SET f = 4.5 WHERE a = 3").unwrap();
            db.execute("DELETE FROM fb WHERE a = 3").unwrap();
            assert_eq!(
                one(db.execute("SELECT a FROM fb ORDER BY a").unwrap())
                    .rows
                    .len(),
                2,
                "{label}: the table lost rows to a failed index build"
            );

            // And the name is free.
            db.execute("CREATE INDEX fb_ix ON fb (a)").unwrap();
            assert!(db.schema().get_index("fb_ix").is_some(), "{label}");
        }
    }

    #[test]
    fn a_refused_constraint_index_leaves_nothing_on_the_add_column_and_pkey_routes() {
        // `create_unique_index` is reached from four call sites — `execute_create_table`,
        // `execute_alter_table_add_column`, `add_primary_key`, `add_unique` — and a
        // constraint's index is a B-tree over bare columns, whose every refusal is the
        // executor's, raised ahead of the storage build. The one that is pglet's own —
        // and so the last, behind every gate standing in for an 18.6 error — is the
        // page bound: a key whose encoding outgrows a B+tree page where 18.6's layout
        // holds it (`EDGES.md` row 192), here an `int[]` of zeros, each element's key
        // carrying `0x00` bytes the framing escapes.
        //
        // - `execute_alter_table_add_column` does **not** stage its mutation — the
        //   column is pushed into the catalog and `storage.add_column` runs before the
        //   index loop — so the pre-flight's position ahead of that block is the whole
        //   of what keeps the route clean, and it is what the first leg locks.
        // - `add_primary_key` raises it from its build scan, ahead of the storage
        //   build and of every catalog write.
        //
        // One row per table, because the *unique* pre-validation (`23505`) sits ahead of
        // the bound and a constant `DEFAULT` over two rows would trip it first.
        let zeros = format!("'{{{}}}'", vec!["0"; 600].join(","));
        for (label, mut db) in [
            ("btree", Database::with_btree()),
            ("memory", Database::in_memory()),
        ] {
            // `ALTER TABLE … ADD COLUMN … UNIQUE`: the column, its default and its
            // index all ride the same statement, and none of them may land.
            db.execute("CREATE TABLE rc (a INT)").unwrap();
            db.execute("INSERT INTO rc VALUES (1)").unwrap();
            let before = db.schema().next_oid;
            let err = db
                .execute(&format!(
                    "ALTER TABLE rc ADD COLUMN f INT[] UNIQUE DEFAULT {zeros}"
                ))
                .unwrap_err();
            assert_eq!(err.sqlstate(), "54000", "{label}");
            assert!(err.to_string().contains("B+tree page"), "{label}: {err}");
            assert_eq!(
                db.schema().get_table("rc").unwrap().columns.len(),
                1,
                "{label}: the refused ADD COLUMN left a phantom column"
            );
            assert!(db.schema().get_index("rc_f_key").is_none(), "{label}");
            assert_eq!(db.schema().next_oid, before, "{label}");
            assert_eq!(one(db.execute("SELECT * FROM rc").unwrap()).rows.len(), 1);
            // The column name is free and the ordinary form of the statement lands.
            db.execute("ALTER TABLE rc ADD COLUMN f INT[]").unwrap();

            // `ALTER TABLE … ADD PRIMARY KEY`, whose index is the fourth route.
            db.execute("CREATE TABLE rp (f INT[] NOT NULL)").unwrap();
            db.execute(&format!("INSERT INTO rp VALUES ({zeros})"))
                .unwrap();
            let before = db.schema().next_oid;
            let err = db
                .execute("ALTER TABLE rp ADD PRIMARY KEY (f)")
                .unwrap_err();
            assert_eq!(err.sqlstate(), "54000", "{label}");
            assert!(db.schema().get_index("rp_pkey").is_none(), "{label}");
            assert_eq!(db.schema().next_oid, before, "{label}");
            db.execute("INSERT INTO rp VALUES ('{1}')").unwrap();
            assert_eq!(
                one(db.execute("SELECT f FROM rp").unwrap()).rows.len(),
                2,
                "{label}: the table lost writability to a refused PRIMARY KEY build"
            );
        }
    }

    #[test]
    fn a_failed_create_index_build_inside_a_block_leaves_nothing_to_roll_back() {
        // The transaction shape of the same defect, and the one where the two halves
        // used to point opposite ways. `ROLLBACK` restores the catalog snapshot, so the
        // catalog half went away on its own; the storage def did not, because
        // `StorageAdapter::create_index` notes an id for the undo only when the call
        // returns `Ok`. The table's writes then failed against an index no statement
        // could name — `DROP INDEX` answered `42704` — until the process restarted.
        //
        // With the build all-or-nothing there is nothing to note and nothing to shed:
        // the assertion is that the writes after the rollback behave as they do on 18.6.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE tb (a INT)").unwrap();
        db.execute("INSERT INTO tb VALUES (0), (1)").unwrap();
        db.execute("BEGIN").unwrap();
        let err = db
            .execute("CREATE INDEX tb_ix ON tb ((10 / a))")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "22012");
        db.execute("ROLLBACK").unwrap();

        // The row the failed expression key chokes on is the one that proves it: an
        // `INSERT` of another `0` was `22012` where 18.6 accepts it.
        db.execute("INSERT INTO tb VALUES (0)").unwrap();
        db.execute("INSERT INTO tb VALUES (2)").unwrap();
        assert_eq!(
            one(db.execute("SELECT a FROM tb ORDER BY a").unwrap())
                .rows
                .len(),
            4
        );
        // The name is free rather than held by a def only the storage knows about.
        assert!(db.schema().get_index("tb_ix").is_none());
        db.execute("CREATE INDEX tb_ix ON tb (a)").unwrap();
    }

    #[test]
    fn an_index_oid_is_never_reused_after_its_index_is_dropped() {
        // The counter write both index routes now own — `next_object_oid` reads the
        // number and the route writes `schema.next_oid` back on its success path — is
        // load-bearing for exactly one property, and it is not the one that looks
        // obvious. Intra-statement distinctness survives without it: `next_object_oid`
        // re-seeds from the live catalog max, so `execute_create_table`'s constraint
        // loop reads past the index the previous iteration registered whether or not
        // the counter moved — measured by deleting the write in `create_unique_index`
        // and watching `CREATE TABLE mc (a INT PRIMARY KEY, b INT UNIQUE, c INT)` still
        // mint three distinct OIDs. What the write buys is
        // monotonicity **across a `DROP`**, where the max no longer holds the number —
        // and a reused index OID puts two logical indexes on one storage band, so
        // dropping one orphans the other (`generate_object_oid`).
        //
        // Both routes, since each writes the counter for itself: `CREATE INDEX` and a
        // declared constraint's index.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE oz (a INT, b INT)").unwrap();
        let mut seen = vec![db.schema().get_table("oz").unwrap().oid];

        db.execute("CREATE INDEX oz_ix ON oz (a)").unwrap();
        seen.push(db.schema().get_index("oz_ix").unwrap().oid);
        db.execute("DROP INDEX oz_ix").unwrap();

        db.execute("ALTER TABLE oz ADD CONSTRAINT oz_u UNIQUE (b)")
            .unwrap();
        let constraint_oid = db.schema().get_index("oz_u").unwrap().oid;
        assert!(
            !seen.contains(&constraint_oid),
            "a constraint index reused a dropped OID: {constraint_oid} in {seen:?}"
        );
        seen.push(constraint_oid);
        db.execute("DROP TABLE oz").unwrap();

        db.execute("CREATE TABLE oz2 (a INT PRIMARY KEY, b INT UNIQUE)")
            .unwrap();
        db.execute("CREATE INDEX oz2_ix ON oz2 (b)").unwrap();
        let mut after: Vec<i32> = db.schema().indexes.values().map(|i| i.oid).collect();
        after.push(db.schema().get_table("oz2").unwrap().oid);
        after.sort_unstable();
        let mut distinct = after.clone();
        distinct.dedup();
        assert_eq!(after, distinct, "two live objects share one OID: {after:?}");
        for oid in &after {
            assert!(
                !seen.contains(oid),
                "a new object reused the dropped OID {oid} (dropped: {seen:?})"
            );
        }
    }

    #[test]
    fn a_create_unique_index_over_duplicates_leaves_no_index_and_no_oid() {
        // The build gate's **catalog** half, which no fixture can assert: the harness
        // runs every fixture inside `BEGIN … ROLLBACK`, so a phantom index and a burned
        // OID both vanish before anything can read them, and an `@expect-error`
        // fixture's own statement is the last one its transaction runs.
        //
        // The gate sits ahead of the storage build, where 18.6 raises this `23505`
        // (`docs/ddl.md`), and the reason it must not sit *below* `schema.create_index`
        // is the reason the whole family exists: the autocommit path rolls storage back
        // and leaves the catalog mutated, so a check that ran after it would leave a
        // durable phantom whose name is `42P07` forever. The counter half of this
        // assertion no longer rides on the gate's position — the statement reads the
        // number with `next_object_oid` and writes `schema.next_oid` only on its success
        // path, so `before` holds however late the gate runs.
        //
        // Both storage backends, because they answer differently on the *other* half of
        // this class — the memory double never rolls back — and a gate that ran too
        // late would look clean on one of them.
        for mut db in [Database::with_btree(), Database::in_memory()] {
            db.execute("CREATE TABLE cud (a INT, b INT)").unwrap();
            db.execute("INSERT INTO cud VALUES (1, 7), (2, 7)").unwrap();
            let before = db.schema().next_oid;
            let err = db
                .execute("CREATE UNIQUE INDEX cud_ix ON cud (b)")
                .unwrap_err();
            assert_eq!(err.sqlstate(), "23505");
            assert!(
                db.schema().get_index("cud_ix").is_none(),
                "a refused index build left a phantom index"
            );
            assert_eq!(
                db.schema().next_oid,
                before,
                "a refused index build moved the OID counter"
            );
            // The name is still free, and the same statement succeeds once the
            // duplicate is gone — the refusal changed nothing.
            db.execute("DELETE FROM cud WHERE a = 2").unwrap();
            db.execute("CREATE UNIQUE INDEX cud_ix ON cud (b)").unwrap();
        }
    }

    #[test]
    fn a_constraint_index_named_after_its_own_table_leaves_no_phantom() {
        // The third route into the same hole, and the one with the right SQLSTATE
        // already: a declared constraint name equal to the table's own name is 42P07 on
        // both engines, but pglet raised it from `create_unique_index` — after
        // `schema.create_table` — so the name was burned. The relation namespace has to
        // hold the table *before* its constraint indexes are named, as Postgres's
        // heap-then-indexes order puts it.
        let mut db = Database::with_btree();
        let err = db
            .execute("CREATE TABLE selfn (a INT, CONSTRAINT selfn PRIMARY KEY (a))")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42P07");
        assert!(db.schema().get_table("selfn").is_none());
        db.execute("CREATE TABLE selfn (a INT)").unwrap();
    }

    #[test]
    fn a_serial_sequence_named_like_a_constraint_leaves_no_phantom() {
        // A `SERIAL`'s sequence is created *before* the table's constraint indexes
        // (`transformCreateStmt`'s before-list), so a declared constraint name equal to
        // the sequence's is `42P07` — measured on 18.6. That much a fixture pins.
        //
        // What a fixture structurally cannot pin is *where* the error is raised. The
        // sequence names this statement minted are seeded into `ConstraintNames`, so
        // the collision is caught read-only. Without the seed the error still lands —
        // from `create_unique_index`'s backstop, which runs **after**
        // `schema.create_table` and after the sequences are inserted — and the
        // autocommit path cannot put either back: the table name is `42P07` forever,
        // every read of it is `42P01`, and the sequence is servable. The differential
        // harness sees none of that, because every fixture runs inside
        // `BEGIN … ROLLBACK`.
        let mut db = Database::with_btree();
        let err = db
            .execute("CREATE TABLE seqfirst (a SERIAL, CONSTRAINT seqfirst_a_seq PRIMARY KEY (a))")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42P07");
        assert!(
            db.schema().get_table("seqfirst").is_none(),
            "the failed CREATE TABLE left a phantom table"
        );
        assert!(
            !db.schema().sequences.contains_key("seqfirst_a_seq"),
            "the failed CREATE TABLE left a phantom sequence"
        );

        // The behavioral half: both names are still free and the retry works.
        db.execute("CREATE TABLE seqfirst (a SERIAL, b INT)")
            .unwrap();
        db.execute("INSERT INTO seqfirst (b) VALUES (1)").unwrap();
        assert_eq!(
            seq_val(&one(db
                .execute("SELECT nextval('seqfirst_a_seq')")
                .unwrap())),
            2
        );
    }

    #[test]
    fn an_add_column_serial_named_like_a_constraint_leaves_no_phantom() {
        // The `ADD COLUMN` twin of `a_serial_sequence_named_like_a_constraint_leaves_
        // no_phantom`, and it needs its own guard for the same reason the `CREATE
        // TABLE` one does: without the `seed_relations` call the `42P07` still fires —
        // from `create_unique_index`'s backstop, which runs *after* the sequence insert
        // and the column push — so the SQLSTATE a fixture compares is identical either
        // way and only the catalog residue differs. A per-family fix looks complete
        // because the gate is shared; it isn't, because each family seeds separately.
        //
        // `transformAlterTableStmt` schedules the `CreateSeqStmt` into the before-list
        // here too, so the sequence takes `d1_y_seq` and the declared index name
        // collides with it (`42P07`, measured on 18.6).
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE d1 (x INT)").unwrap();
        let err = db
            .execute("ALTER TABLE d1 ADD COLUMN y SERIAL CONSTRAINT d1_y_seq UNIQUE")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42P07");
        assert!(
            !db.schema().sequences.contains_key("d1_y_seq"),
            "the failed ADD COLUMN left a phantom sequence"
        );
        assert!(
            db.schema()
                .get_table("d1")
                .is_some_and(|t| t.columns.iter().all(|c| c.name != "y")),
            "the failed ADD COLUMN left a phantom column"
        );

        // The behavioral half: both names are still free and the retry works.
        db.execute("ALTER TABLE d1 ADD COLUMN y SERIAL").unwrap();
        db.execute("INSERT INTO d1 (x) VALUES (1)").unwrap();
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('d1_y_seq')").unwrap())),
            2
        );
    }

    #[test]
    fn a_failed_add_column_backfill_leaves_no_phantom_sequence() {
        // The `ADD COLUMN` sequence registration used to run **before** the constraint
        // validations (`validate_added_column_uniques`, `validate_added_column_checks`,
        // `validate_added_column_fk`, and before the `23502` backfill gate), so any of
        // the three violations they raise — `23514` / `23505` / `23503` — left the sequence
        // durably in the catalog with the backfill's own advances on its counter. Storage rolls back and `self.schema` does not, so
        // `nextval('pht_c_seq')` then answered **3** where 18.6 answers `42P01`, and the
        // name stayed claimed forever. (`23502` is a gate on the same path and not a
        // fourth violation: a `NextVal` default resolves to an `int8` that no coercion
        // makes NULL, so `ADD COLUMN c SERIAL NOT NULL` succeeds on both engines.)
        // No fixture can carry this: a fixture's one `@test` is compared on its last
        // statement's rows, so it cannot fail a statement and then probe the catalog —
        // an `@expect-error` stops at the SQLSTATE. The isolating `BEGIN … ROLLBACK`
        // would restore the snapshot too, but that is a second reason rather than the
        // reason, and `@no-txn` fixtures skip the wrapper.
        //
        // Measured on 18.6: the `ALTER` is `23514`, `nextval('pht_c_seq')` is `42P01`,
        // `CREATE SEQUENCE pht_c_seq` then succeeds and answers `1`, and `pht` still
        // has its one column and both rows.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE pht (id INT)").unwrap();
        db.execute("INSERT INTO pht VALUES (1), (2)").unwrap();

        let err = db
            .execute("ALTER TABLE pht ADD COLUMN c SERIAL CHECK (c > 100)")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "23514", "{err}");
        assert!(
            !db.schema().sequences.contains_key("pht_c_seq"),
            "the failed ADD COLUMN left a phantom sequence in the catalog"
        );
        // Not servable — the behavioral half a catalog-only assertion would miss.
        assert_eq!(
            db.execute("SELECT nextval('pht_c_seq')")
                .unwrap_err()
                .sqlstate(),
            "42P01"
        );
        // The column never landed and both rows are intact.
        assert!(
            db.schema()
                .get_table("pht")
                .is_some_and(|t| t.columns.len() == 1 && t.columns[0].name == "id"),
            "the failed ADD COLUMN left a phantom column"
        );
        assert_eq!(
            one(db.execute("SELECT id FROM pht ORDER BY id").unwrap())
                .rows
                .len(),
            2
        );
        // And the name is unclaimed: the sequence a user creates under it is theirs,
        // starting where they asked rather than resuming a ghost's counter.
        db.execute("CREATE SEQUENCE pht_c_seq").unwrap();
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('pht_c_seq')").unwrap())),
            1
        );

        // The success path is unchanged, and the counter still carries the backfill's
        // draws: two rows take 1 and 2, so the next `nextval` answers 3.
        db.execute("CREATE TABLE pht_ok (id INT)").unwrap();
        db.execute("INSERT INTO pht_ok VALUES (1), (2)").unwrap();
        db.execute("ALTER TABLE pht_ok ADD COLUMN c SERIAL")
            .unwrap();
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('pht_ok_c_seq')").unwrap())),
            3
        );
    }

    #[test]
    fn a_failed_add_column_leaves_no_phantom_user_named_sequence() {
        // The other half of the same registration: a user's own `DEFAULT nextval('x')`
        // naming a sequence the catalog lacks, which pglet **auto-creates** at DDL time
        // (18.6 resolves `'x'::regclass` in parse analysis instead and answers `42P01`,
        // the recorded divergence). The auto-creation is the engine's, so a statement
        // that then fails must take it back — otherwise a name nobody created is
        // durably a sequence, and the dump emits a `CREATE SEQUENCE` for it.
        //
        // Kept distinct from the SERIAL case above because they are two arms of the
        // registration `match`, and the `23502` gate reaches this one from a third
        // direction (`DEFAULT nextval(…)` resolves non-NULL, so this arm needs a real
        // constraint violation to fail on).
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE phu (id INT)").unwrap();
        db.execute("INSERT INTO phu VALUES (1), (2)").unwrap();

        let err = db
            .execute("ALTER TABLE phu ADD COLUMN c INT DEFAULT nextval('ghost_addcol_seq') CHECK (c > 100)")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "23514", "{err}");
        assert!(
            !db.schema().sequences.contains_key("ghost_addcol_seq"),
            "the failed ADD COLUMN left a phantom user-named sequence"
        );
        assert_eq!(
            db.execute("SELECT nextval('ghost_addcol_seq')")
                .unwrap_err()
                .sqlstate(),
            "42P01"
        );
        assert!(
            db.schema()
                .get_table("phu")
                .is_some_and(|t| t.columns.len() == 1)
        );

        // A sequence the statement did **not** create keeps its counter, advances and
        // all: `nextval` is non-transactional, so the draws the failed backfill made
        // stand. Staging must not roll an existing counter back — that would re-issue
        // ids already handed out — which is why both arms of the staging `match` skip a
        // name the catalog already holds.
        //
        // **What is shared with 18.6 is the rule, not the number — and only on the
        // row-level constraint checks.** Those 18.6 applies as it rewrites row by row,
        // stopping at the offending one, while pglet resolves the whole backfill and
        // *then* validates it, so pglet's counter carries a draw per existing row.
        // Measured on both over four rows: a `CHECK` failing on row 1 leaves 18.6's next
        // `nextval` at 2 and pglet's at 5, and failing on row 3 leaves 18.6's at 4 and
        // pglet's at 5. The FK and unique routes agree instead (18.6 validates those
        // after the whole rewrite, so both engines land on 5). Loud on neither engine,
        // and it is a gap in a counter that
        // promises no gaplessness — recorded in `docs/compatibility.md`'s Sequences row rather than
        // pinned by a fixture, which would be red.
        //
        // **The line that splits the diverging shapes from the agreeing ones is where
        // the failure happens, not which family raises it** — and an expression default
        // puts members on both sides of it. A failure *inside* the resolution loop stops
        // both engines at the same row, so the counters agree; a failure *after* it does
        // not. Measured on both over three rows and a fresh sequence, every SQLSTATE
        // identical:
        //   - `DEFAULT 100/(nextval('s') - 2)` → `22012`, next `3` on **both** (the
        //     divide aborts row 2's own resolution here and its own rewrite there);
        //   - `DEFAULT nextval('s') CHECK (c > 1000)` → `23514`, pglet `4` vs 18.6 `2`;
        //   - `NOT NULL DEFAULT nullif(nextval('s'), 2)` → `23502`, pglet `4` vs 18.6
        //     `3` — the `23502` gate is per-tuple in `ATRewriteTable` (`:6465`) exactly
        //     as the CHECK list below it is, so it belongs on the diverging side, and a
        //     spelling whose NULL lands on the *last* row (`nullif(…, 3)`) hides that by
        //     making both engines draw three times.
        // Same class, same cause, and the extra members are recorded here rather than
        // pinned for the same reason: a faithful fixture would be red.
        db.execute("CREATE SEQUENCE live_addcol_seq").unwrap();
        let err = db
            .execute("ALTER TABLE phu ADD COLUMN d INT DEFAULT nextval('live_addcol_seq') CHECK (d > 100)")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "23514", "{err}");
        assert_eq!(
            seq_val(&one(db
                .execute("SELECT nextval('live_addcol_seq')")
                .unwrap())),
            3
        );
    }

    #[test]
    fn a_refused_add_column_unique_index_leaves_the_table_whole() {
        // The fourth fallible step, and the one the staging comment used to list as an
        // absence: the inline `UNIQUE`'s index build ran *after* the sequence merge and
        // the column push, so a key the build refuses raised on top of a mutated
        // catalog. That is not the phantom-sequence shape but the **wire-brick** one —
        // the column and its index land, the widened rows roll back with storage, and
        // every `SELECT *` then answers two descriptors over one-value rows, which a
        // client reads as `unexpected field count in "D" message`, permanently and
        // across a restart.
        //
        // The refusal is pglet's own, so it is the last check ahead of the mutation
        // block: the page bound over the back-filled **values** — here an `int[]` whose
        // key outgrows a B+tree page (each `0` element's key carries bytes the framing
        // escapes) where 18.6's layout holds it and the `ALTER` succeeds (`EDGES.md`
        // row 192). What this pins is that the over-rejection costs nothing but the
        // refusal.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE jb (id INT)").unwrap();
        db.execute("INSERT INTO jb VALUES (1), (2)").unwrap();
        let zeros = vec!["0"; 600].join(", ");
        let err = db
            .execute(&format!(
                "ALTER TABLE jb ADD COLUMN c INT[] \
                 DEFAULT ARRAY[nextval('jb_fresh')::int, {zeros}] UNIQUE"
            ))
            .unwrap_err();
        assert_eq!(err.sqlstate(), "54000", "{err}");
        assert!(err.to_string().contains("B+tree page"), "{err}");
        assert!(
            !db.schema().sequences.contains_key("jb_fresh"),
            "the failed ADD COLUMN left a phantom sequence"
        );
        assert_eq!(
            db.execute("SELECT nextval('jb_fresh')")
                .unwrap_err()
                .sqlstate(),
            "42P01"
        );
        assert!(
            db.schema()
                .get_table("jb")
                .is_some_and(|t| t.columns.len() == 1),
            "the failed ADD COLUMN left a phantom column"
        );
        assert!(
            db.schema().indexes.is_empty(),
            "the failed ADD COLUMN left a phantom index"
        );
        // The wire-brick observable itself: descriptors and row width agree, so a
        // client can still read the table.
        let after = one(db.execute("SELECT * FROM jb").unwrap());
        assert_eq!(after.columns().len(), 1);
        assert_eq!(after.rows.len(), 2);
        assert!(after.rows.iter().all(|r| r.len() == 1));

        // The `jsonb` spelling of the statement, a key type the encoder once refused:
        // it lands as it does on 18.6, index and sequence with it, and the index
        // holds — `1.0` is the `1` the first row drew.
        db.execute("CREATE TABLE jz (id INT)").unwrap();
        db.execute("INSERT INTO jz VALUES (1), (2)").unwrap();
        db.execute(
            "ALTER TABLE jz ADD COLUMN c JSONB DEFAULT (nextval('jz_fresh')::text)::jsonb UNIQUE",
        )
        .unwrap();
        assert!(db.schema().get_index("jz_c_key").is_some());
        let dup = db.execute("INSERT INTO jz VALUES (3, '1.0')").unwrap_err();
        assert_eq!(dup.sqlstate(), "23505", "{dup}");

        // The `float8` spelling of the same statement, whose key the encoder takes: it
        // lands as it does on 18.6 (measured there: `1.5` and `3`, and the sequence's
        // next value `3`), index and sequence with it.
        db.execute("CREATE TABLE fz (id INT)").unwrap();
        db.execute("INSERT INTO fz VALUES (1), (2)").unwrap();
        db.execute("ALTER TABLE fz ADD COLUMN c FLOAT8 DEFAULT nextval('fz_fresh') * 1.5 UNIQUE")
            .unwrap();
        assert!(db.schema().get_index("fz_c_key").is_some());
        let rows = one(db.execute("SELECT c FROM fz ORDER BY id").unwrap()).rows;
        assert_eq!(
            rows,
            vec![vec![Value::Float8(1.5)], vec![Value::Float8(3.0)]]
        );
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('fz_fresh')").unwrap())),
            3
        );
        let dup = db.execute("INSERT INTO fz VALUES (3, 1.5)").unwrap_err();
        assert_eq!(dup.sqlstate(), "23505", "{dup}");

        // An all-NULL back-fill encodes nothing: a `jsonb` unique column with no
        // `DEFAULT` is accepted here exactly as on 18.6.
        db.execute("CREATE TABLE fn_ok (id INT)").unwrap();
        db.execute("INSERT INTO fn_ok VALUES (1), (2)").unwrap();
        db.execute("ALTER TABLE fn_ok ADD COLUMN c JSONB UNIQUE")
            .unwrap();
    }

    #[test]
    fn an_add_column_default_may_draw_a_live_and_a_fresh_sequence_at_once() {
        // The multi-sequence widening: an expression default can name several
        // sequences, and the staged (this statement's own) and live (already in the
        // catalog) halves are **not** disjoint, so the backfill draws from one working
        // map holding both. Selecting between them — `if staged_sequences.is_empty()`,
        // the shape that predated `ColumnDefault::Expr` — answers `42P01` for whichever
        // half it did not pick, and no fixture can see it: a differential fixture must
        // `CREATE` both sequences for 18.6, which puts both on the live path.
        //
        // Measured on 18.6, same statements: `101 | 202 | 303`, `nextval('ac_live')`
        // then `4` and `nextval('ac_fresh')` then `4` (18.6 needs the second sequence
        // created in setup; pglet auto-creates it, the recorded divergence).
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE ac (id INT)").unwrap();
        db.execute("INSERT INTO ac VALUES (1), (2), (3)").unwrap();
        db.execute("CREATE SEQUENCE ac_live").unwrap();

        db.execute(
            "ALTER TABLE ac ADD COLUMN c INT DEFAULT nextval('ac_live') * 100 + nextval('ac_fresh')",
        )
        .unwrap();
        let rows = one(db.execute("SELECT c FROM ac ORDER BY id").unwrap()).rows;
        assert_eq!(
            rows,
            vec![
                vec![Value::Int4(101)],
                vec![Value::Int4(202)],
                vec![Value::Int4(303)],
            ]
        );

        // The live half resumed in place (its three draws stand) and the fresh half was
        // registered carrying its own.
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('ac_live')").unwrap())),
            4
        );
        assert!(db.schema().sequences.contains_key("ac_fresh"));
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('ac_fresh')").unwrap())),
            4
        );
    }

    #[test]
    fn a_failed_set_default_leaves_no_phantom_sequence() {
        // The same autocommit-phantom class as the three tests above, on a sequence.
        // `execute_alter_table_set_default` registered the `nextval` sequence *before*
        // resolving the table and the column, so a `SET DEFAULT nextval('ghost')` on a
        // column that does not exist raised its `42703` over a sequence that was now
        // durably in the catalog: `nextval('ghost')` served values and the dump emitted
        // a `CREATE SEQUENCE` for it.
        //
        // This runs on `Database` (the autocommit bracket, not an inner seam) because
        // that is where the class lives — `run_planned_env` rolls back *storage* and
        // leaves `self.schema` mutated. The differential fixtures cannot see it at all:
        // they run inside `BEGIN … ROLLBACK`, which restores the snapshot.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE ph_seq (a INT)").unwrap();

        let err = db
            .execute("ALTER TABLE ph_seq ALTER COLUMN nosuch SET DEFAULT nextval('ghost_seq')")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42703");
        assert!(
            !db.schema().sequences.contains_key("ghost_seq"),
            "the failed ALTER left a phantom sequence in the catalog"
        );

        // The behavioral half, and the one a catalog-only assertion would miss: the
        // ghost is not servable, and the name is still free.
        assert_eq!(
            db.execute("SELECT nextval('ghost_seq')")
                .unwrap_err()
                .sqlstate(),
            "42P01"
        );
        db.execute("CREATE SEQUENCE ghost_seq START WITH 5")
            .unwrap();
        assert_eq!(
            seq_val(&one(db.execute("SELECT nextval('ghost_seq')").unwrap())),
            5
        );

        // The same statement over a missing *table* is likewise clean (the other
        // lookup, and the one that runs first).
        let err = db
            .execute("ALTER TABLE no_such_table ALTER COLUMN a SET DEFAULT nextval('ghost2_seq')")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42P01");
        assert!(!db.schema().sequences.contains_key("ghost2_seq"));
    }

    #[test]
    fn a_declared_sequence_name_equal_to_its_table_names_the_table_and_yields_to_a_sibling() {
        // Which *relation* the `42P07` names is not decorative here — it is the whole
        // difference between the two ways of closing this hole, and the fixture harness
        // compares SQLSTATEs, not messages, so no fixture can see it.
        //
        // Postgres creates the before-list sequence successfully and finds the clash in
        // `heap_create_with_catalog`, so the message names the **table**. Seeding the
        // table's own name into the naming loop instead answers the same `42P07` and
        // names the *sequence* — and, worse, at the wrong position: the whole
        // before-list runs before the heap, so a later column colliding with an
        // already-taken name wins. Both measured on 18.6, in both written orders.
        let mut db = Database::with_btree();
        let err = db
            .execute("CREATE TABLE selfseq (id INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME selfseq))")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42P07");
        assert_eq!(err.to_string(), "relation \"selfseq\" already exists");
        assert!(
            db.schema().get_table("selfseq").is_none(),
            "the failed CREATE TABLE left a phantom table"
        );
        assert!(
            !db.schema().sequences.contains_key("selfseq"),
            "the failed CREATE TABLE left a phantom sequence"
        );

        // A sibling declaring an *already-taken* name outranks it, in either order.
        db.execute("CREATE SEQUENCE selfseq_taken").unwrap();
        for stmt in [
            "CREATE TABLE selfseq (x INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME selfseq), \
             y INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME selfseq_taken))",
            "CREATE TABLE selfseq (y INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME selfseq_taken), \
             x INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME selfseq))",
        ] {
            let err = db.execute(stmt).unwrap_err();
            assert_eq!(err.sqlstate(), "42P07");
            assert_eq!(
                err.to_string(),
                "relation \"selfseq_taken\" already exists",
                "the sibling's collision outranks the table's own name"
            );
        }

        // The behavioral half: the name is still free and the retry works.
        db.execute("CREATE TABLE selfseq (a SERIAL, b INT)")
            .unwrap();
    }

    #[test]
    fn a_default_nextval_naming_its_own_table_is_the_wrong_kind() {
        // Postgres cooks a column default once the heap exists, so `'ownref'::regclass`
        // binds the **table** and every write is `42809`; pglet has no regclass and
        // auto-creates the sequence, so it raises the same `42809` at DDL — the
        // documented over-rejection, spelled with the SQLSTATE Postgres reaches rather
        // than the `42P07` the declared-name half takes. The fixture cannot pin that
        // choice (Postgres accepts the statement, so the harness accepts any error).
        //
        // Accepting it is what this guards: the catalog then held `ownref` as a table
        // *and* a sequence, `nextval('ownref')` answered `42809` while
        // `INSERT … VALUES (DEFAULT)` returned 1, and the dump emitted a
        // `CREATE SEQUENCE ownref` ahead of the `CREATE TABLE ownref` that neither
        // engine reloads — losing the table and every row.
        let mut db = Database::with_btree();
        let err = db
            .execute("CREATE TABLE ownref (b INT DEFAULT nextval('ownref'), c INT)")
            .unwrap_err();
        assert_eq!(err.sqlstate(), "42809");
        assert!(db.schema().get_table("ownref").is_none());
        assert!(!db.schema().sequences.contains_key("ownref"));

        // And it is only the statement's *own* table: a free name still auto-creates.
        db.execute("CREATE TABLE ownok (b INT DEFAULT nextval('ownok_seq'), c INT)")
            .unwrap();
        assert!(db.schema().sequences.contains_key("ownok_seq"));
    }

    #[test]
    fn a_sibling_default_does_not_overwrite_an_implicit_sequence_record() {
        // `CREATE TABLE k (b INT DEFAULT nextval('k_a_seq'), a SERIAL)` is accepted by
        // both engines — the before-list has created `k_a_seq` by the time the default
        // is cooked, so the two columns share it. pglet's "is this sequence already
        // there?" test reads the catalog, which cannot see the sequence this same
        // statement claimed, so it registered a second, bare record; written first, the
        // bare one won the `or_insert` and erased both the identity column's
        // `START WITH` and its ownership edge. The start half a fixture can read; the
        // owner half is observable only in the dump.
        let mut db = Database::with_btree();
        db.execute(
            "CREATE TABLE ksib (b INT DEFAULT nextval('ksib_a_seq'), \
             a INT GENERATED ALWAYS AS IDENTITY (START WITH 100))",
        )
        .unwrap();
        let seq = db.schema().sequences.get("ksib_a_seq").unwrap();
        assert_eq!(seq.start, 100, "the identity column's START WITH survived");
        let owner = seq.owner.as_ref().expect("the ownership edge survived");
        assert_eq!((owner.table.as_str(), owner.column.as_str()), ("ksib", "a"));

        // The dump renders both facts, and this is the shape that lost them. They ride
        // the identity column's own statement now: `ADD GENERATED` creates the sequence
        // under its declared name, seeds it, and records the ownership edge in one go,
        // so a `CREATE SEQUENCE` beside it would be `42P07` and an `ALTER SEQUENCE …
        // OWNED BY` beside it `0A000`.
        let dump = db.dump_sql().unwrap();
        assert!(
            dump.contains(
                "ALTER TABLE ksib ALTER COLUMN a ADD GENERATED ALWAYS AS IDENTITY \
                 (SEQUENCE NAME ksib_a_seq START WITH 100);"
            ),
            "the dump lost the declared start (or the sequence's name):\n{dump}"
        );
        assert!(
            !dump.contains("CREATE SEQUENCE ksib_a_seq"),
            "the ADD GENERATED creates it; a second creation is 42P07:\n{dump}"
        );
        assert!(
            !dump.contains("OWNED BY"),
            "the ADD GENERATED records the edge; re-stating it is 0A000:\n{dump}"
        );
        // ...and the sibling column drawing from that same sequence is hoisted out of
        // the `CREATE TABLE`, which now runs *before* the sequence exists.
        assert!(
            dump.contains("ALTER TABLE ksib ALTER COLUMN b SET DEFAULT nextval('ksib_a_seq'"),
            "the sibling default must survive the hoist:\n{dump}"
        );
        assert!(
            dump.find("ADD GENERATED").unwrap() < dump.find("SET DEFAULT").unwrap(),
            "the default names a sequence the ADD GENERATED creates:\n{dump}"
        );
    }

    #[test]
    fn sequence_advance_survives_rollback_btree() {
        // A sequence advance is non-transactional (matches Postgres): the counter
        // moved by nextval is not restored by ROLLBACK on the B+tree backend.
        let mut db = Database::with_btree();
        db.execute("CREATE SEQUENCE s").unwrap();
        db.execute("SELECT nextval('s')").unwrap(); // → 1
        db.begin(0).unwrap();
        db.execute("SELECT nextval('s')").unwrap(); // → 2, inside the txn
        db.rollback(0).unwrap();
        // The advance survives: the next value is 3, not 2.
        assert_eq!(seq_val(&one(db.execute("SELECT nextval('s')").unwrap())), 3);
    }

    /// A statement's answer on `conn` through the bound entry point (no parameters), as
    /// its command tag or `ERR <SQLSTATE> <message>`.
    fn bound(db: &mut Database, sql: &str, conn: i32) -> String {
        match db.execute_bound(sql, conn, |_| Ok::<_, ()>(Vec::new())) {
            Ok(result) => result.command_tag,
            Err(BoundExecError::Engine(e)) => format!("ERR {} {e}", e.sqlstate()),
            Err(BoundExecError::Bind(())) => "bind".to_owned(),
        }
    }

    /// The same through `execute_as`: the last result's tag, or the error.
    fn simple(db: &mut Database, sql: &str, conn: i32) -> String {
        match db.execute_as(sql, conn) {
            Ok(results) => results
                .last()
                .map_or_else(|| "(none)".to_owned(), |r| r.command_tag.clone()),
            Err(e) => format!("ERR {} {e}", e.sqlstate()),
        }
    }

    fn raw_parses() -> usize {
        RAW_PARSES.with(Cell::get)
    }

    fn catalog_encodes() -> usize {
        CATALOG_ENCODES.with(Cell::get)
    }

    /// A raw parse comes ahead of everything after it, so a lexical error is the
    /// statement's answer before the aborted-block gate, a session command and parse
    /// analysis — measured on 18.6 with verbatim simple queries: an unterminated literal
    /// in an aborted block is `42601`, not `25P02`; after a relation that does not exist
    /// it is still `42601`, not `42P01`; and a `SHOW` or `SET` holding one is `42601`.
    /// A valid statement in an aborted block is `25P02`, its `COMMIT` the tag `ROLLBACK`,
    /// and a text of comments alone is no statement. Both entry points, on texts with no
    /// `;` byte, whose raw parse waits (`raw_parse_can_wait`), and on the same texts
    /// with one, whose does not.
    ///
    /// Answers the parse-first route (`raw_parse_can_wait`) must keep, so green with it
    /// taken out; red with a failed parse answered as it failed rather than taken the
    /// long way (`42P01` ahead of the literal).
    #[test]
    fn a_lexical_error_outranks_the_gate_a_session_command_and_analysis() {
        let unterminated = "unterminated string literal";
        let entries: [fn(&mut Database, &str, i32) -> String; 2] = [bound, simple];
        for (terminator, label) in [("", "no `;`"), (";", "a `;`")] {
            for entry in entries {
                let mut db = Database::with_btree();
                let mut ask = |sql: &str| entry(&mut db, &format!("{sql}{terminator}"), 1);
                assert!(
                    ask("SELECT * FROM nosuch WHERE a = 'abc").starts_with("ERR 42601"),
                    "{label}: ahead of analysis"
                );
                let answer = ask("SELECT * FROM nosuch WHERE a = 'abc");
                assert!(answer.contains(unterminated), "{label}: {answer}");
                assert!(
                    ask("SHOW search_path 'abc").starts_with("ERR 42601"),
                    "{label}: ahead of a SHOW"
                );
                assert!(
                    ask("SET search_path TO 'abc").starts_with("ERR 42601"),
                    "{label}: ahead of a SET"
                );
                // pglet's lex answers the literal where 18.6's streaming grammar answers
                // `syntax error at or near "FROM"`, both `42601`; the long way keeps it.
                assert!(
                    ask("SELECT FROM FROM 'abc").contains(unterminated),
                    "{label}"
                );
                assert!(
                    ask("SELECT * FROM nosuch").starts_with("ERR 42P01"),
                    "{label}"
                );
                assert_eq!(ask("BEGIN"), "BEGIN", "{label}");
                assert!(ask("SELECT 1/0").starts_with("ERR 22012"), "{label}");
                assert!(
                    ask("SELECT 'abc").starts_with("ERR 42601"),
                    "{label}: ahead of the gate"
                );
                assert!(ask("/* c").starts_with("ERR 42601"), "{label}");
                assert!(ask("SELECT 1").starts_with("ERR 25P02"), "{label}");
                assert!(ask("SHOW search_path").starts_with("ERR 25P02"), "{label}");
                assert_eq!(ask("COMMIT"), "ROLLBACK", "{label}");
                assert_eq!(db.transaction_status(), TransactionStatus::Idle, "{label}");
            }
        }
        // A piece of comments alone: no statement to the count, so the bound entry point
        // answers its parse's count, and `execute_as` runs nothing.
        let mut db = Database::with_btree();
        for sql in ["-- c", "/* c */", "/* c */;"] {
            assert_eq!(
                bound(&mut db, sql, 1),
                "ERR 42601 syntax error: expected 1 statement, got 0",
                "{sql}"
            );
            assert_eq!(simple(&mut db, sql, 1), "(none)", "{sql}");
        }
        // Two statements: refused whole by the bound entry point, in a block too, which it
        // aborts; run one after the other by `execute_as`.
        assert_eq!(
            bound(&mut db, "SELECT 1; SELECT 2", 1),
            "ERR 42601 syntax error: expected 1 statement, got 2"
        );
        assert_eq!(bound(&mut db, "BEGIN", 1), "BEGIN");
        assert_eq!(
            bound(&mut db, "SELECT 1; SELECT 2", 1),
            "ERR 42601 syntax error: expected 1 statement, got 2"
        );
        assert!(bound(&mut db, "SELECT 1", 1).starts_with("ERR 25P02"));
        assert_eq!(
            bound(&mut db, "COMMIT; SELECT 1", 1),
            "ERR 42601 syntax error: expected 1 statement, got 2"
        );
        assert_eq!(bound(&mut db, "ROLLBACK", 1), "ROLLBACK");
        assert_eq!(simple(&mut db, "SELECT 1; SELECT 2", 1), "SELECT 1");
        assert_eq!(
            simple(&mut db, "BEGIN; SELECT 1/0", 1).get(..9),
            Some("ERR 22012")
        );
        assert_eq!(simple(&mut db, "COMMIT; SELECT 1", 1), "SELECT 1");
    }

    /// Every session command is one the parser refuses, which is what lets a text be
    /// parsed ahead of the interception that services it (`raw_parse_can_wait`): a parse
    /// that succeeded would run a statement the long way hands to the dispatcher. Each is
    /// answered the dispatcher's way through both entry points, with and without a `;`.
    ///
    /// Red with the parser accepting any of them.
    #[test]
    fn a_session_command_is_never_parsed_ahead_of_its_interception() {
        let commands = [
            ("SET search_path TO public", "SET"),
            ("set lock_timeout = '2s'", "SET"),
            ("SET LOCAL lock_timeout = '2s'", "SET"),
            ("SET TIME ZONE 'UTC'", "SET"),
            ("SET NAMES 'UTF8'", "SET"),
            ("SET TRANSACTION ISOLATION LEVEL READ COMMITTED", "SET"),
            (
                "SET SESSION CHARACTERISTICS AS TRANSACTION ISOLATION LEVEL READ COMMITTED",
                "SET",
            ),
            ("SHOW search_path", "SHOW"),
            ("SHOW ALL", "SHOW"),
            ("RESET lock_timeout", "RESET"),
            ("RESET ALL", "RESET"),
            ("DISCARD ALL", "DISCARD ALL"),
            ("DEALLOCATE ALL", "DEALLOCATE ALL"),
            ("DEALLOCATE PREPARE ALL", "DEALLOCATE ALL"),
        ];
        let schema = Schema::new();
        for (sql, tag) in commands {
            assert!(pglet_sql::catalog::is_session_command(sql), "{sql}");
            assert!(pglet_sql::parser::parse(sql, &schema).is_err(), "{sql}");
            let mut db = Database::with_btree();
            for terminator in ["", ";"] {
                let text = format!("{sql}{terminator}");
                assert_eq!(bound(&mut db, &text, 1), tag, "{text}");
                assert_eq!(simple(&mut db, &text, 1), tag, "{text}");
            }
        }
    }

    /// A text with no `;` byte outside an aborted block is lexed once, by its parse: the
    /// facade's raw parse runs for neither entry point. It still runs where it decides
    /// something — a `;` in the text, an aborted block — and after a parse that fails,
    /// a session command's among them.
    ///
    /// Red with the raw parse taken back to every statement (`raw_parse_can_wait`
    /// answering `false`).
    #[test]
    fn a_statement_with_no_semicolon_is_lexed_once() {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (a INT)").unwrap();
        let before = raw_parses();
        assert_eq!(bound(&mut db, "INSERT INTO t VALUES (1)", 1), "INSERT 0 1");
        assert_eq!(bound(&mut db, "SELECT a FROM t", 1), "SELECT 1");
        assert_eq!(simple(&mut db, "SELECT a FROM t", 1), "SELECT 1");
        assert_eq!(bound(&mut db, "BEGIN", 1), "BEGIN");
        assert_eq!(bound(&mut db, "COMMIT", 1), "COMMIT");
        assert_eq!(raw_parses(), before, "no raw parse for a text with no `;`");
        assert_eq!(bound(&mut db, "SHOW search_path", 1), "SHOW");
        assert_eq!(simple(&mut db, "SHOW search_path", 1), "SHOW");
        assert!(bound(&mut db, "SELECT nosuch FROM t", 1).starts_with("ERR 42703"));
        assert_eq!(
            raw_parses(),
            before + 3,
            "the long way, after a failed parse"
        );
        let before = raw_parses();
        assert_eq!(bound(&mut db, "SELECT a FROM t;", 1), "SELECT 1");
        assert_eq!(simple(&mut db, "SELECT a FROM t;", 1), "SELECT 1");
        assert_eq!(raw_parses(), before + 2, "a `;` is counted");
        assert_eq!(bound(&mut db, "BEGIN", 1), "BEGIN");
        assert!(bound(&mut db, "SELECT 1/0", 1).starts_with("ERR 22012"));
        let aborted = raw_parses();
        assert!(bound(&mut db, "SELECT 1", 1).starts_with("ERR 25P02"));
        assert!(simple(&mut db, "SELECT 1", 1).starts_with("ERR 25P02"));
        assert_eq!(raw_parses(), aborted + 2, "an aborted block lexes first");
        assert_eq!(bound(&mut db, "ROLLBACK", 1), "ROLLBACK");
    }

    /// A store's commits that change the catalog — DDL, a `SERIAL` draw, `nextval`, a
    /// block's `COMMIT` — encode it where something reads what its write leaves, and
    /// nowhere else. An in-memory B+tree its one connection drives encodes nothing, its
    /// tree takes no catalog, and a catalog-only commit leaves its commit stamp where it
    /// was; another connection's block or write is refused there, and its reads run.
    /// The in-memory B+tree any connection may drive encodes as it did, because its stamp
    /// is what another connection's `REPEATABLE READ` block compares: `nextval` and a
    /// `CREATE SEQUENCE` there refuse that block's first write `40001` (`EDGES.md` row
    /// 232). The durable control is `tests/sequence_catalog.rs`'s.
    ///
    /// Red with `StorageBackend::persists_catalog` answering `true` for every B+tree:
    /// the one-connection store encodes, and its stamp moves on a `nextval`.
    #[test]
    fn a_store_one_connection_drives_encodes_no_catalog() {
        const STATEMENTS: [&str; 6] = [
            "CREATE TABLE t (id SERIAL, v INT)",
            "CREATE SEQUENCE s",
            "INSERT INTO t (v) VALUES (1)",
            "SELECT nextval('s')",
            "BEGIN",
            "INSERT INTO t (v) VALUES (2)",
        ];
        let stamp = |db: &Database| db.storage().commit_stamp();
        let tree_catalog = |db: &Database| match db.storage() {
            StorageBackend::Btree(btree) => btree.persisted_catalog().unwrap().len(),
            #[cfg(feature = "memory_storage")]
            StorageBackend::Memory(_) => unreachable!(),
        };

        let mut db = Database::with_btree_for_one_connection(0);
        let before = catalog_encodes();
        for sql in STATEMENTS {
            db.execute(sql).unwrap();
        }
        db.execute("COMMIT").unwrap();
        assert_eq!(catalog_encodes(), before, "nothing reads the encoding");
        assert_eq!(tree_catalog(&db), 0, "no catalog write");
        let at = stamp(&db);
        assert_eq!(seq_val(&one(db.execute("SELECT nextval('s')").unwrap())), 2);
        assert_eq!(stamp(&db), at, "a catalog-only commit moves nothing");
        assert_eq!(
            one(db.execute("SELECT count(*) FROM t").unwrap()).rows[0][0],
            Value::Int8(2)
        );
        for sql in [
            "BEGIN",
            "INSERT INTO t (v) VALUES (3)",
            "SELECT nextval('s')",
        ] {
            let e = db.execute_as(sql, 3).unwrap_err();
            assert_eq!(e.sqlstate(), "0A000", "{sql}: {e}");
            assert!(
                e.to_string()
                    .contains("other than the one this database serves"),
                "{sql}: {e}"
            );
        }
        assert_eq!(
            one(db.execute_as("SELECT count(*) FROM t", 3).unwrap()).rows[0][0],
            Value::Int8(2),
            "another connection's read runs"
        );
        let mut fork = db.fork().unwrap();
        assert!(
            fork.execute_as("BEGIN", 3).is_err(),
            "a fork keeps the one connection"
        );
        let before = catalog_encodes();
        fork.execute("SELECT nextval('s')").unwrap();
        assert_eq!(catalog_encodes(), before, "nor does a fork encode");

        // The shared store: encodes, and its stamp is read across connections.
        let mut db = Database::with_btree();
        let before = catalog_encodes();
        for sql in STATEMENTS {
            db.execute(sql).unwrap();
        }
        db.execute("COMMIT").unwrap();
        assert!(catalog_encodes() > before);
        assert!(tree_catalog(&db) > 0);
        for other in ["SELECT nextval('s')", "CREATE SEQUENCE s2"] {
            assert_eq!(
                simple(&mut db, "BEGIN ISOLATION LEVEL REPEATABLE READ", 4),
                "BEGIN"
            );
            assert_eq!(
                simple(&mut db, "SELECT v FROM t WHERE id = 1", 4),
                "SELECT 1"
            );
            assert!(!simple(&mut db, other, 3).starts_with("ERR"), "{other}");
            assert!(
                simple(&mut db, "UPDATE t SET v = 0 WHERE id = 1", 4).starts_with("ERR 40001"),
                "{other}"
            );
            assert_eq!(simple(&mut db, "ROLLBACK", 4), "ROLLBACK");
        }
    }

    /// A Unicode space is a name character to 18.6's scanner, so a decision about
    /// statement text skips and splits on the lexer's whitespace alone
    /// (`pglet_sql::lexer::is_space`) — measured on 18.6, each answer below is its own:
    ///
    /// - `execute_as`'s parse-first route reads the bytes the raw lex reads:
    ///   `SELECT 1` and then a no-break space, or a next-line character, is `42601`
    ///   (trailing junk after a numeric literal), in a block too, which it aborts;
    /// - the statement split keeps a trailing ideographic space on its piece:
    ///   `DROP TABLE t` and then one is `42P01` and drops nothing, and a leading
    ///   no-break space is `42601`;
    /// - the `SET`/`SHOW` classifier: a leading no-break space is `42601`, not the
    ///   setting, and a tab, line feed or vertical tab after the keyword is the command;
    /// - the `SHOW` name reader: a trailing ideographic space or an inner no-break space
    ///   is part of the name, `42704`.
    ///
    /// Red for each with `str::trim` / `char::is_whitespace` (or a `"SET "` literal)
    /// back at its site.
    #[test]
    fn a_decision_about_statement_text_skips_only_the_lexers_whitespace() {
        let entries: [fn(&mut Database, &str, i32) -> String; 2] = [bound, simple];
        for trailing in ["SELECT 1\u{a0}", "SELECT 1\u{85}"] {
            for block in [false, true] {
                let mut db = Database::with_btree();
                if block {
                    assert_eq!(simple(&mut db, "BEGIN", 1), "BEGIN");
                }
                let answer = simple(&mut db, trailing, 1);
                assert!(answer.starts_with("ERR 42601"), "{block}: {answer}");
                let next = simple(&mut db, "SELECT 1", 1);
                assert_eq!(next.starts_with("ERR 25P02"), block, "{block}: {next}");
            }
        }
        for terminator in ["", ";"] {
            for entry in entries {
                let mut db = Database::with_btree();
                db.execute("CREATE TABLE t (a INT)").unwrap();
                let mut ask = |sql: &str| entry(&mut db, &format!("{sql}{terminator}"), 1);
                assert!(ask("DROP TABLE t\u{3000}").starts_with("ERR 42P01"));
                assert_eq!(ask("SELECT a FROM t"), "SELECT 0", "t stands");
                assert!(ask("\u{a0}SELECT 1").starts_with("ERR 42601"));
                assert!(ask("\u{a0}SET lock_timeout = '3s'").starts_with("ERR 42601"));
                assert!(ask("\u{a0}SHOW lock_timeout").starts_with("ERR 42601"));
                for spaced in ["SET\tlock_timeout = '3s'", "SET\nlock_timeout = '3s'"] {
                    assert_eq!(ask(spaced), "SET", "{spaced:?}");
                }
                assert_eq!(ask("SET\u{b}lock_timeout = '4s'"), "SET");
                assert_eq!(ask("SHOW\tlock_timeout"), "SHOW");
                let shown = ask("SHOW lock_timeout\u{3000}");
                assert!(shown.starts_with("ERR 42704"), "{shown}");
                assert!(shown.contains("lock_timeout\u{3000}"), "{shown}");
                let shown = ask("SHOW lock\u{a0}timeout");
                assert!(shown.contains("lock\u{a0}timeout"), "{shown}");
            }
        }
    }
}
