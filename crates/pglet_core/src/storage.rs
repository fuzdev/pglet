//! The single `Storage` trait shared across the workspace.
//!
//! Every backend (the in-memory and B+tree impls in `pglet_storage`) and every
//! executor-facing adapter (in `pglet`, `pglet_wasm`) implements this one trait,
//! and the SQL executor (`pglet_sql`) is generic over it. It lives in the leaf
//! crate so both the producers (`pglet_storage`) and the consumer (`pglet_sql`)
//! can name it without a dependency cycle.

use crate::Result;
use crate::spec::{IndexSpec, RowId};
use crate::types::Value;

/// Storage backend trait.
///
/// Implementations provide persistence for table data.
/// The catalog (schema metadata) is managed separately.
///
/// # Row IDs
///
/// Each row is assigned a unique `RowId` on insert. This enables:
/// - ID-keyed DELETE/UPDATE (address a row by its stable ID, not its full value)
/// - Index entries address a row by its `RowId`
/// - Clean separation between predicate evaluation (executor) and storage
pub trait Storage: Send + Sync {
    /// Creates a new table.
    ///
    /// # Errors
    ///
    /// Returns error if table already exists.
    fn create_table(&mut self, name: &str) -> Result<()>;

    /// Drops a table.
    ///
    /// # Errors
    ///
    /// Returns error if table doesn't exist.
    fn drop_table(&mut self, name: &str) -> Result<()>;

    /// Inserts a row into a table. Returns the assigned row ID.
    ///
    /// # Errors
    ///
    /// Returns error if table doesn't exist.
    fn insert(&mut self, table: &str, row: Vec<Value>) -> Result<RowId>;

    /// Scans all rows from a table. Returns (`row_id`, `row_values`) pairs.
    ///
    /// # Errors
    ///
    /// Returns error if table doesn't exist.
    fn scan(&self, table: &str) -> Result<Vec<(RowId, Vec<Value>)>>;

    /// Gets a single row by ID.
    ///
    /// # Errors
    ///
    /// Returns error if table doesn't exist.
    fn get(&self, table: &str, id: RowId) -> Result<Option<Vec<Value>>>;

    /// Deletes a row by ID. Returns the deleted row if it existed.
    ///
    /// # Errors
    ///
    /// Returns error if table doesn't exist.
    fn delete(&mut self, table: &str, id: RowId) -> Result<Option<Vec<Value>>>;

    /// Updates a row by ID. Returns the old row values if it existed.
    ///
    /// # Errors
    ///
    /// Returns error if table doesn't exist.
    fn update(&mut self, table: &str, id: RowId, new_row: Vec<Value>)
    -> Result<Option<Vec<Value>>>;

    /// Adds a column to all rows in a table with a default value.
    ///
    /// # Errors
    ///
    /// Returns error if table doesn't exist.
    fn add_column(&mut self, table: &str, default: Value) -> Result<()>;

    /// Drops a column from all rows in a table by index.
    ///
    /// # Errors
    ///
    /// Returns error if table doesn't exist.
    fn drop_column(&mut self, table: &str, column_index: usize) -> Result<()>;

    /// Registers a secondary index and back-fills entries for existing rows.
    ///
    /// `spec` carries the index id, table, key-column positions, uniqueness
    /// intent, the partial-index predicate (serialized planner `Expr` IR,
    /// deserialized + cached here), the NULL-distinctness rule, and the table's
    /// column names the predicate evaluator binds against (see `IndexSpec`).
    /// Single-column and composite keys are supported.
    ///
    /// After this returns, the index is maintained on every subsequent
    /// insert/update/delete on `table` and is probeable via `index_lookup`.
    ///
    /// **All-or-nothing**: a call that returns an error registers nothing — no
    /// definition, no entries, no per-table maintenance list membership — so the
    /// caller's catalog write may follow the build rather than precede it. It is a
    /// contract rather than an observation, and it is what the executor's ordering
    /// rests on: the autocommit path rolls **storage** back on an error and leaves the
    /// catalog mutated, so a `CREATE INDEX` that wrote the catalog first and then
    /// failed the back-fill left a durable phantom whose name was `42P07` forever,
    /// while the B+tree backend's surviving definition went on refusing writes to the
    /// table against an index no statement could name. The residual is the write
    /// itself, as on `rebuild_index`: a B+tree `apply_batch` that fails part-way is an
    /// I/O error rather than a refused encoding, and this contract does not reach
    /// inside it. What that leaves is **pages, not entries** — `apply_batch` publishes
    /// the new root once, after the page writes and the data fsync both, so a reader
    /// still descends the pre-batch root and nothing the failed batch wrote is
    /// reachable from it. The cost is a leak: those pages stay written and the store
    /// never reclaims them. Measured over both halves of that tail — a page write
    /// that fails mid-loop, and a fsync that fails with every page written — by
    /// `pglet`'s `a_failed_back_fill_write_leaves_no_readable_entry_in_the_band`, which
    /// reads the band back through a later index that takes the same id.
    ///
    /// # Errors
    ///
    /// Returns an error if the table doesn't exist, a column position is out of
    /// range, an indexed value is not B-tree-indexable, the partial predicate
    /// fails to parse, or back-fill fails.
    fn create_index(&mut self, spec: IndexSpec) -> Result<()>;

    /// Reattaches a secondary index definition WITHOUT back-filling entries.
    ///
    /// This is the reopen path: the index entries are already persisted in the
    /// tree (they share the data root and survive in the meta slot), so all that
    /// is lost on restart is the in-memory def map that drives maintenance and
    /// probe validation. `reattach_index` restores only that def — it performs
    /// no tree writes and assumes the entries are already present. Use
    /// `create_index` instead when the entries do not yet exist (fresh
    /// `CREATE INDEX`), which back-fills from the existing rows.
    ///
    /// Arguments mirror `create_index`.
    ///
    /// # Errors
    ///
    /// Returns an error if the table doesn't exist or the partial predicate fails
    /// to parse.
    fn reattach_index(&mut self, spec: IndexSpec) -> Result<()>;

    /// Re-registers an index that **already exists**, putting both its definition and
    /// its entries back in step with a catalog fact that moved underneath it.
    ///
    /// The fact it was built for is an enum's member list: `IndexSpec::column_enum_orders`
    /// is a copy (this crate holds no catalog to borrow from) and `DROP TYPE` +
    /// `CREATE TYPE` can move what it copied, which for a **partial** index whose
    /// predicate orders an enum — or an expression key over one — changes which rows
    /// belong in the index. See `docs/enums.md` § A copy that outlives its statement.
    ///
    /// The spec's **other** parallel lists (`column_names`, `column_types`,
    /// `key_col_positions`) are settled rather than open, and neither column DDL reaches
    /// them: `ADD COLUMN` appends, leaving every earlier position where the spec put it,
    /// and `DROP COLUMN` is *refused* — `executor::ddl`'s `index_affected_by_column_drop`
    /// answers `0A000` for any index on the table carrying a predicate, an expression
    /// key, or a key column at or after the dropped position. That refusal is
    /// deliberately wider than the stale-position case its message names (a partial index
    /// is refused whatever its key's position, and 18.6 accepts that drop), so it is an
    /// over-rejection as well as a guard — measured, `EDGES.md` row 108, and not relaxed
    /// here.
    ///
    /// It is neither of the two neighbours, and both of those are wrong here:
    /// `reattach_index` swaps the definition but leaves entries the old rule placed,
    /// and on the in-memory backend it drops every entry (its defs *own* their
    /// entries); `drop_index` + `create_index` rebuilds correctly on that backend and
    /// **not** on the B+tree, where a drop taken inside a transaction is deferred to
    /// commit and deregisters the definition the create had just restored — measured,
    /// as an `index not found` on the next write, and the reason this is a method
    /// rather than a two-call idiom at the caller.
    ///
    /// Idempotent, and derived entirely from `spec` plus the table's current rows, so
    /// it may be re-applied whenever the catalog moves — including a rollback's restore
    /// moving it *back*.
    ///
    /// **All-or-nothing**: a rebuild that returns an error leaves the index exactly as
    /// it was, definition and entries alike, so a caller moving several indexes at once
    /// has to put back only the ones that *succeeded*. It is a contract rather than an
    /// observation — the in-memory backend dropped the old definition before building
    /// the new one, so a refused key encoding deleted the index from storage while the
    /// catalog still carried it. The residual is the write itself: a B+tree `apply_batch`
    /// that fails part-way is an I/O error rather than a refused encoding, and this
    /// contract does not reach inside it.
    ///
    /// # Errors
    ///
    /// Returns an error if the table doesn't exist, the partial predicate or a key
    /// expression fails to parse, an indexed value has no B-tree encoding, or the
    /// rebuild fails.
    fn rebuild_index(&mut self, spec: IndexSpec) -> Result<()>;

    /// Drops a secondary index: removes its definition and all its entries.
    ///
    /// A no-op (Ok) if no index with `index_id` is registered.
    ///
    /// # Errors
    ///
    /// Returns an error if removing the index entries fails.
    fn drop_index(&mut self, index_id: u32) -> Result<()>;

    /// Equality probe: returns the row IDs whose indexed columns equal `values`.
    ///
    /// `values` is the per-column key tuple in the index's column order — one
    /// element for a single-column index, N for a composite (multi-column) index.
    /// The result is the same row set a full `scan` + per-column filter would
    /// yield (order unspecified). If **any** key value is NULL the probe returns
    /// no rows (SQL equality never matches NULL; under the default `NULLS
    /// DISTINCT` a NULL in any key column is never a uniqueness candidate).
    ///
    /// # Errors
    ///
    /// Returns an error if the index doesn't exist or a value is not
    /// B-tree-indexable.
    fn index_lookup(&self, index_id: u32, values: &[Value]) -> Result<Vec<RowId>>;

    /// Whether a **different live connection**'s uncommitted write already claims the
    /// key tuple `values` in `index_id`.
    ///
    /// The half of a uniqueness decision [`Storage::index_lookup`] structurally cannot
    /// answer. A probe sees committed rows plus its *own* pending writes, so a key
    /// another connection is holding inside an open block is invisible to it and both
    /// writers commit the same key. Postgres reaches the same verdict by making the
    /// second writer wait on the first's row lock; pglet's native server makes it wait
    /// for the block holding the writer and the embedded `Database` refuses it `55P03`,
    /// so under them this question has no other connection's write to find. It is kept
    /// as the storage layer's own guard: a hit refuses the later writer at its statement,
    /// and a backend counts it (the B+tree's `cross_connection_hits`).
    ///
    /// A claim is **not a row** — the claiming write may still roll back — so it is
    /// carried as its own fact rather than as a phantom [`RowId`] mixed into a lookup
    /// result. That separation is what keeps every *read* path (`IndexScan`, the range
    /// and containment probes, the foreign-key existence probe) seeing only what its
    /// own snapshot holds.
    ///
    /// The default is `false`, so a backend that buffers no cross-connection pending
    /// writes — the in-memory store, the no-op below, a snapshot reader — keeps its
    /// behaviour exactly.
    ///
    /// # Errors
    ///
    /// Returns an error if the index doesn't exist or a value is not
    /// B-tree-indexable.
    fn unique_key_claimed_elsewhere(&self, _index_id: u32, _values: &[Value]) -> Result<bool> {
        Ok(false)
    }

    /// Ordered range probe over a **single-column** index.
    ///
    /// Returns the row ids whose indexed value is **non-NULL** and falls in the range
    /// bounded by `lower`/`upper`, **in index-key order** — ascending when `ascending`,
    /// else descending — limited to `limit` rows if set. Each bound is
    /// `Some((value, inclusive))` or `None` for no bound that side, so `> x`,
    /// `>= x`, `< x`, `<= x`, `BETWEEN a AND b`, and an ordered scan of every non-NULL
    /// key are all expressible. With a bound, the result is the same row **set** a full
    /// `scan` + range filter would yield, ordered as requested.
    ///
    /// NULL handling: a comparison is not true of a NULL (SQL `NULL <op> x` is
    /// `unknown`), so **no result holds a row whose key is NULL** — whether the index
    /// holds an entry for one (`NULLS NOT DISTINCT`) or does not (the default), and
    /// whether or not a bound was given. A lookup with no bound therefore answers every
    /// row of the table only where the key cannot be NULL. A NULL *bound value* is no
    /// comparison either: it is treated as no bound that side, and a caller that means
    /// `col <op> NULL` answers no row without asking.
    ///
    /// # Errors
    ///
    /// Returns an error if the index doesn't exist or a bound value is not
    /// B-tree-indexable.
    fn index_range_lookup(
        &self,
        index_id: u32,
        lower: Option<(Value, bool)>,
        upper: Option<(Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<RowId>>;

    /// Containment probe over a **GIN** index: the row IDs whose indexed value
    /// `@>`-contains **every** decomposed query key in `keys`.
    ///
    /// `keys` is the containment constant already decomposed into its element/token
    /// set (`pglet_sql::gin::gin_decompose` — array elements, or JSONB key/value
    /// tokens). The result is the intersection of each key's posting list: a row
    /// appears only when it holds *all* the keys. Because GIN is **lossy** (JSONB
    /// containment, NULL array elements), the result is a **superset** of the true
    /// `@>` matches — the caller MUST recheck each row with the real `@>` predicate.
    ///
    /// An **empty** `keys` returns no rows. `x @> '{}'` / `x @> ARRAY[]` is true for
    /// every row, which the index can't accelerate, so the caller handles that case
    /// with a full scan rather than calling this.
    ///
    /// # Errors
    ///
    /// Returns an error if the index doesn't exist.
    fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> Result<Vec<RowId>>;

    /// Establish a storage savepoint: everything written from here on can be
    /// discarded on its own, without discarding what came before it.
    ///
    /// # A top-of-stack API, and why it carries no name
    ///
    /// SQL savepoints are named, may repeat a name, and `ROLLBACK TO`/`RELEASE`
    /// take the most recent frame of that name. **None of that is here.** The names
    /// live in exactly one place — `pglet::TransactionSession`, which also holds the
    /// per-frame catalog snapshot a storage backend knows nothing about — and it
    /// drives this stack in lockstep, one push per frame it pushes and one pop per
    /// frame it drops. A named API here would be the same rule written twice, and
    /// the two copies would decide *which frame* independently.
    ///
    /// The second consumer is the one that could not use a named API at all: a `DO`
    /// block wraps itself in a level whose name no SQL text ever mentions — the
    /// subtransaction an `EXCEPTION` clause gives its body, and the statement's own undo
    /// when it fails.
    ///
    /// # Errors
    ///
    /// Returns the backend's error if it cannot stage one — for a backend whose
    /// writes are only durable, `0A000`.
    fn savepoint(&mut self) -> Result<()>;

    /// Discard everything written since the most recent savepoint, **keeping** the
    /// savepoint.
    ///
    /// Keeping it is Postgres's rule and not an implementation convenience:
    /// `ROLLBACK TO s` is repeatable, so `SAVEPOINT s; INSERT …; ROLLBACK TO s;
    /// INSERT …; ROLLBACK TO s` is legal and leaves neither insert. A pop-to-restore
    /// implementation answers `3B001` to the second one.
    ///
    /// # Errors
    ///
    /// Returns the backend's error if there is no savepoint to roll back to, or if
    /// the restore cannot complete.
    fn rollback_to_savepoint(&mut self) -> Result<()>;

    /// Drop the most recent savepoint, **keeping** everything written since it — the
    /// sub-work merges into the enclosing level.
    ///
    /// # Errors
    ///
    /// Returns the backend's error if there is no savepoint to release.
    fn release_savepoint(&mut self) -> Result<()>;

    /// Whether every row of `table` was written within 18.6's TOAST threshold, as far as
    /// this store has watched — so no value 18.6 would hold for it is compressed or
    /// stored out of line, and `pg_column_size` can answer from the value alone.
    ///
    /// **`false` is "not known", and it is the default.** A store answers `true` only for
    /// a table it created and has seen every row write to since: a write past the
    /// threshold ([`Self::note_toasted_row`]), a column added with a non-NULL default
    /// over existing rows (18.6 reads those rows' values from `attmissingval`, under a
    /// four-byte header) and a dropped column (whose bytes 18.6's old rows keep) each end
    /// it for good, and a store reopened from disk knows it of no table. Being
    /// conservative costs a refusal, never an answer: a store that does not track it
    /// leaves the default, and `pg_column_size` refuses there.
    fn rows_untoasted(&self, _table: &str) -> bool {
        false
    }

    /// Record that a row of `table` was written whose 18.6 layout is past the TOAST
    /// threshold — or could not be sized — so [`Self::rows_untoasted`] answers `false`
    /// for it from now on. Monotonic: nothing but recreating the table clears it, so a
    /// mark a later rollback strands costs a refusal and never an answer.
    fn note_toasted_row(&self, _table: &str) {}
}

/// The one question `pg_column_size` asks of a store.
///
/// [`Storage::rows_untoasted`] behind a trait object, so expression evaluation can hold
/// it without being generic over the store.
pub trait ToastStatus {
    /// [`Storage::rows_untoasted`].
    fn rows_untoasted(&self, table: &str) -> bool;
}

impl<S: Storage> ToastStatus for S {
    fn rows_untoasted(&self, table: &str) -> bool {
        Storage::rows_untoasted(self, table)
    }
}

/// No-op storage for tests and the storage-free `execute` entry point.
#[derive(Debug)]
pub struct NoopStorage;

impl Storage for NoopStorage {
    fn create_table(&mut self, _name: &str) -> Result<()> {
        Ok(())
    }
    fn drop_table(&mut self, _name: &str) -> Result<()> {
        Ok(())
    }
    fn insert(&mut self, _table: &str, _row: Vec<Value>) -> Result<RowId> {
        Ok(0)
    }
    fn scan(&self, _table: &str) -> Result<Vec<(RowId, Vec<Value>)>> {
        Ok(Vec::new())
    }
    fn get(&self, _table: &str, _id: RowId) -> Result<Option<Vec<Value>>> {
        Ok(None)
    }
    fn delete(&mut self, _table: &str, _id: RowId) -> Result<Option<Vec<Value>>> {
        Ok(None)
    }
    fn update(
        &mut self,
        _table: &str,
        _id: RowId,
        _new_row: Vec<Value>,
    ) -> Result<Option<Vec<Value>>> {
        Ok(None)
    }
    fn add_column(&mut self, _table: &str, _default: Value) -> Result<()> {
        Ok(())
    }
    fn drop_column(&mut self, _table: &str, _column_index: usize) -> Result<()> {
        Ok(())
    }
    fn create_index(&mut self, _spec: IndexSpec) -> Result<()> {
        Ok(())
    }
    fn reattach_index(&mut self, _spec: IndexSpec) -> Result<()> {
        Ok(())
    }
    fn rebuild_index(&mut self, _spec: IndexSpec) -> Result<()> {
        Ok(())
    }
    fn drop_index(&mut self, _index_id: u32) -> Result<()> {
        Ok(())
    }
    fn index_lookup(&self, _index_id: u32, _values: &[Value]) -> Result<Vec<RowId>> {
        Ok(Vec::new())
    }
    fn index_range_lookup(
        &self,
        _index_id: u32,
        _lower: Option<(Value, bool)>,
        _upper: Option<(Value, bool)>,
        _ascending: bool,
        _limit: Option<usize>,
    ) -> Result<Vec<RowId>> {
        Ok(Vec::new())
    }
    fn gin_lookup(&self, _index_id: u32, _keys: &[Value]) -> Result<Vec<RowId>> {
        Ok(Vec::new())
    }
    // The no-op storage accepts every write and keeps none, so the three savepoint
    // operations are vacuously satisfied — refusing here would fail every `DO` on the
    // storage-free `execute` entry point, whose body cannot have written anything for a
    // rollback to discard.
    fn savepoint(&mut self) -> Result<()> {
        Ok(())
    }
    fn rollback_to_savepoint(&mut self) -> Result<()> {
        Ok(())
    }
    fn release_savepoint(&mut self) -> Result<()> {
        Ok(())
    }
}
