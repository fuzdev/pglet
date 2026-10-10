//! Core types for query execution.

use crate::catalog::{Index, IndexKey, Table};
use crate::types::Value;
use crate::{Result, planner::serialize_expr};

// Re-export ColumnDesc and ResultShape from types module
pub use crate::types::{ColumnDesc, ResultShape};

// `RowId`, `IndexSpec`, and `IndexMethod` live in the `pglet_core` leaf crate;
// re-export them so the `executor::{RowId, IndexSpec, IndexMethod}` paths resolve.
pub use pglet_core::{IndexMethod, IndexSpec, RowId};

/// Query result with column names and rows.
#[derive(Debug, Clone)]
pub struct QueryResult {
    /// Whether this statement returned a **result set**, and its columns when it did.
    ///
    /// Carried as a [`ResultShape`] and not a column list because the two facts are
    /// independent: `SELECT *` over a relation whose last column was dropped returns a
    /// result set of **zero** columns, which every wire reply has to tell from a
    /// statement — a DDL, a `RETURNING`-less DML — that returns no result set at all.
    /// Reading `columns.is_empty()` for that decision is `EDGES.md` row 59: the simple
    /// leg dropped the `RowDescription` *and the rows with it*, reporting `SELECT 2`
    /// and sending none.
    ///
    /// [`Self::columns`] is the list for readers that only need names and types.
    pub shape: ResultShape,
    pub rows: Vec<Vec<Value>>,
    pub command_tag: String,
    /// `NOTICE` messages this statement raised, emitted before `CommandComplete`
    /// by the wire server. Postgres notices a no-op skip (`DROP TABLE IF EXISTS`
    /// of a missing table → `table "x" does not exist, skipping`); pglet carries
    /// that text here rather than swallowing it silently. Usually empty.
    pub notices: Vec<String>,
    /// The transaction-lifecycle effect this statement calls for, or `None` for
    /// every statement that has none.
    ///
    /// **The signal is carried rather than re-derived from `command_tag`**, because
    /// the tag is not a function of the statement: `ROLLBACK` and
    /// `ROLLBACK TO SAVEPOINT` both report `ROLLBACK`, and a host dispatching on the
    /// tag reads the second as the first — discarding every write since `BEGIN` and
    /// reporting success. It rides `QueryResult` rather than being read off the
    /// `Plan` because the native server plans inside a closure under the schema lock
    /// and only the result comes back out; both hosts then read one field.
    pub tx_command: Option<TransactionCommand>,
    /// The sequence counter cells this statement replaced that its plan could not list
    /// ahead of it, each with the `last_value` the outgoing cell held — empty for every
    /// statement but a `DO` that completed.
    ///
    /// A host reads a statement's replaced cells from `Plan::swapped_sequences` *before*
    /// the statement runs, since the outgoing value is gone after it. A `DO` statement
    /// lists nothing there: which of its statements run, and against which catalog, is
    /// decided as it runs. So the `DO` block collects them from its own statements as it
    /// runs them (`executor::execute_do_block`) and hands them out here, where the host
    /// adds them to what it read ahead and an enclosing `DO` adds them to its own list.
    /// A `DO` that fails puts its replaced cells back itself and reports none. It rides
    /// the result for [`Self::tx_command`]'s reason: the native server executes under
    /// its locks and only the result comes back out.
    pub swapped_sequences: Vec<(String, i64)>,
}

/// What an executed statement asks the session's transaction block to do.
///
/// Produced by the executor's transaction-control arms and consumed by
/// `pglet::TransactionSession`; the SQL front-end itself holds no block state, so
/// this is the whole of what it says about one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransactionCommand {
    /// `BEGIN` / `START TRANSACTION`, with the isolation level its mode list asked for
    /// (`None`: the session's `default_transaction_isolation`).
    Begin(Option<IsolationLevel>),
    /// `COMMIT` / `END`.
    Commit,
    /// `ROLLBACK` / `ABORT` — the whole block.
    Rollback,
    /// `SAVEPOINT <name>`, the name already folded.
    Savepoint(String),
    /// `ROLLBACK TO [SAVEPOINT] <name>`, the name already folded.
    RollbackTo(String),
    /// `RELEASE [SAVEPOINT] <name>`, the name already folded.
    Release(String),
}

/// A transaction isolation level — what `BEGIN … ISOLATION LEVEL`, `SET TRANSACTION`
/// and `default_transaction_isolation` name, spelled as 18.6's `isolation_level_options`
/// spell them.
///
/// What pglet does with each is the host's: a block at [`Self::RepeatableRead`] or
/// [`Self::Serializable`] keeps the snapshot it began with and is refused `40001` at its
/// first write when anything committed since (`pglet::TransactionSession::take_writer`);
/// [`Self::ReadUncommitted`] behaves as [`Self::ReadCommitted`], as on 18.6, and reports
/// its own name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
pub enum IsolationLevel {
    ReadUncommitted,
    ReadCommitted,
    RepeatableRead,
    Serializable,
}

impl IsolationLevel {
    /// Every level, in the order 18.6's hint lists them.
    pub const ALL: [Self; 4] = [
        Self::Serializable,
        Self::RepeatableRead,
        Self::ReadCommitted,
        Self::ReadUncommitted,
    ];

    /// The setting's value as `SHOW` renders it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::ReadUncommitted => "read uncommitted",
            Self::ReadCommitted => "read committed",
            Self::RepeatableRead => "repeatable read",
            Self::Serializable => "serializable",
        }
    }

    /// Read a setting's value — 18.6's `config_enum_lookup_by_name`, which compares
    /// case-insensitively and trims nothing.
    #[must_use]
    pub fn from_setting(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|level| level.name().eq_ignore_ascii_case(value))
    }

    /// Whether a block at this level keeps one snapshot for its whole length — 18.6's
    /// `IsolationUsesXactSnapshot`.
    #[must_use]
    pub const fn uses_transaction_snapshot(self) -> bool {
        matches!(self, Self::RepeatableRead | Self::Serializable)
    }
}

impl QueryResult {
    /// The result set's columns, or an empty slice when the statement returned no
    /// result set. Shorthand for `self.shape.columns()`.
    ///
    /// Do **not** read this to decide whether to send a `RowDescription`, `DataRow`s or
    /// `NoData` — it is empty for both cases. That decision reads
    /// [`ResultShape::is_result_set`].
    #[must_use]
    pub fn columns(&self) -> &[ColumnDesc] {
        self.shape.columns()
    }

    /// Creates an empty query result.
    pub const fn empty() -> Self {
        Self {
            shape: ResultShape::NoResultSet,
            rows: Vec::new(),
            command_tag: String::new(),
            notices: Vec::new(),
            tx_command: None,
            swapped_sequences: Vec::new(),
        }
    }

    /// Creates a DDL result (no result set, just a command tag).
    pub fn ddl(tag: &str) -> Self {
        Self {
            shape: ResultShape::NoResultSet,
            rows: Vec::new(),
            command_tag: tag.to_string(),
            notices: Vec::new(),
            tx_command: None,
            swapped_sequences: Vec::new(),
        }
    }

    /// A transaction-control result: a command tag plus the lifecycle effect the
    /// host applies once the statement has executed.
    pub fn transaction(tag: &str, command: TransactionCommand) -> Self {
        Self {
            tx_command: Some(command),
            ..Self::ddl(tag)
        }
    }

    /// A DDL result carrying a single `NOTICE` message — the `IF EXISTS`-skipped
    /// DROP case (`table "x" does not exist, skipping`), where Postgres notices the
    /// no-op before `CommandComplete`.
    pub fn ddl_with_notice(tag: &str, notice: String) -> Self {
        Self::ddl_with_notices(tag, vec![notice])
    }

    /// A DDL result carrying **several** notices — a multi-name `DROP … IF EXISTS`
    /// raises one per *written* name it skipped, so a list naming the same missing
    /// relation twice notices twice (measured on 18.6). An empty `notices` is the
    /// ordinary [`Self::ddl`] shape.
    pub fn ddl_with_notices(tag: &str, notices: Vec<String>) -> Self {
        Self {
            shape: ResultShape::NoResultSet,
            rows: Vec::new(),
            command_tag: tag.to_string(),
            notices,
            tx_command: None,
            swapped_sequences: Vec::new(),
        }
    }
}

/// Build the storage `IndexSpec` for a catalog `index` over `table`.
///
/// The single catalog → storage hand-off, so every site that registers an
/// index — `CREATE INDEX`, the auto-created PK/UNIQUE indexes, and the reopen
/// re-registration — propagates the same fields (key positions or serialized key
/// expressions, the serialized partial predicate, NULL-distinctness, and the
/// table's column names the predicate/key evaluator binds against).
///
/// For an **expression** index (`IndexKey::Exprs`), each key part is serialized to
/// opaque bytes (`key_part_ir`) and `key_col_positions` is empty; for a **plain**
/// `enums` is the catalog's enum map, from which each column's member order is read —
/// the same `Schema.enums` slice every ordering site reads, so a stored predicate's
/// enum comparison is decided by the one member list at maintenance time too.
///
/// index (`IndexKey::Columns`), each column is resolved to its row position and
/// `key_part_ir` is `None`. Returns `Ok(None)` if a plain key column can no longer
/// be resolved against `table` (a corrupt/stale def — the caller skips it rather
/// than aborting), and `Err` only if serializing a key/predicate `Expr` fails.
///
/// A free function rather than an inherent method because `IndexSpec` now lives
/// in the `pglet_core` leaf crate (which cannot name `catalog::{Index, Table}`).
///
/// # Errors
///
/// Returns `Error::Execution` if a key expression or the partial predicate fails to
/// serialize (does not happen for a valid catalog `Expr`).
pub fn index_spec_from_catalog(
    index: &Index,
    table: &Table,
    enums: &std::collections::BTreeMap<String, crate::catalog::EnumType>,
) -> Result<Option<IndexSpec>> {
    let (key_col_positions, key_part_ir) = match &index.key {
        IndexKey::Columns(cols) => {
            // Resolve each bare column to its row position; a missing column means
            // the index can't match any row, so the whole spec is skipped.
            let Some(positions) = cols
                .iter()
                .map(|col| table.columns.iter().position(|c| &c.name == col))
                .collect::<Option<Vec<usize>>>()
            else {
                return Ok(None);
            };
            (positions, None)
        }
        IndexKey::Exprs(exprs) => {
            let parts = exprs
                .iter()
                .map(serialize_expr)
                .collect::<Result<Vec<Vec<u8>>>>()?;
            (Vec::new(), Some(parts))
        }
    };
    let predicate_ir = index.predicate.as_ref().map(serialize_expr).transpose()?;
    Ok(Some(IndexSpec {
        index_id: index.index_id(),
        name: index.name.clone(),
        table: index.table_name.clone(),
        key_col_positions,
        key_part_ir,
        unique: index.is_unique(),
        predicate_ir,
        nulls_not_distinct: index.nulls_not_distinct,
        column_names: table.columns.iter().map(|c| c.name.clone()).collect(),
        column_types: table.columns.iter().map(|c| c.type_oid).collect(),
        column_enum_orders: table
            .columns
            .iter()
            .map(|c| {
                super::helpers::enum_order_of(enums, c.enum_type.as_deref()).map(<[String]>::to_vec)
            })
            .collect(),
        method: index_method_for(index, table),
    }))
}

/// Derive the access method for `index` from its key shape, its uniqueness, and its
/// key column's type.
///
/// A single bare **non-unique** column whose type is JSONB or an array is a **GIN**
/// index, and `CREATE INDEX` refuses a declared-or-default B-tree over that shape —
/// this derivation is the only record of an index's method, so it cannot answer two
/// ways for one shape (`EDGES.md` row 25). Every other index — scalar, composite,
/// expression, or **unique** — is **B-tree**, so the key shape and uniqueness decide
/// this as much as the column type does; a sentence saying the type determines the
/// method is wrong about the routes this function's own `unique` arm serves. The unique
/// exclusion matters: GIN can't enforce uniqueness, and a real GIN index is validated
/// non-unique at `CREATE`, so a unique index over a container type is a B-tree, keyed
/// by `Value::encode_index_key`. Not persisted; recomputed on every register/reopen.
///
/// **The one answer to "what method is this index", for all three consumers**: the
/// storage `IndexSpec` built here, `pg_catalog`'s `indexdef` rendering (via
/// [`IndexMethod::name`]), and the planner's index selection — whose two selectors
/// (`single_column_index_for` for B-tree, `gin_index_for` for GIN) each gate on it, so the
/// planner cannot offer a decomposed GIN key to an equality probe, a range probe, or an
/// ordered scan. A second derivation anywhere is a drift bug waiting to happen; that is why
/// the method is recomputed from the catalog rather than persisted. Crate-internal:
/// re-exported from `executor` only as `pub(crate)` (see `executor/mod.rs`).
pub fn index_method_for(index: &Index, table: &Table) -> IndexMethod {
    // `single_bare_column` answers only for a single-column plain key — so a
    // composite, expression, or multi-part key never derives GIN here.
    if !index.is_unique()
        && let Some(col_name) = index.key.single_bare_column()
        && let Some(col) = table.columns.iter().find(|c| c.name == col_name)
        && col.type_oid.is_container()
    {
        return IndexMethod::Gin;
    }
    IndexMethod::Btree
}

// The `Storage` trait and `NoopStorage` now live in the `pglet_core` leaf crate
// (`pglet_core::storage`) — one definition shared by the backends and this
// executor. `executor::mod` re-exports them so the `executor::Storage` /
// `executor::NoopStorage` paths still resolve here.
