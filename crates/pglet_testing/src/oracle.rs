//! The in-memory `Storage` oracle the real B+tree backend is differential-tested
//! against, plus the `TestDb` driver and seed helpers shared across the
//! `engine_*` tests.
//!
//! `MemStorage` answers every query by scan-and-filter — it is the source of
//! truth the real (indexed) storage must match, so its index probes are
//! deliberately naive (no real indexing, just predicate-filtered scans).

use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};

use pglet_sql::Result;
use pglet_sql::catalog::Schema;
use pglet_sql::executor::{
    EvalEnv, QueryResult, RowId, SequenceSession, Storage, execute_with_storage,
};
use pglet_sql::parser::parse;
use pglet_sql::planner::{Expr, deserialize_expr};
use pglet_sql::types::{ArrayValue, TypeOid, Value};

/// A registered secondary index (single- or multi-column) for the test store.
#[derive(Debug, Clone)]
struct MemIndexDef {
    table: String,
    /// Bare-column key positions (a plain index). Empty for an expression index,
    /// which keys via `key_exprs` instead.
    key_cols: Vec<usize>,
    /// Key-part expressions (an expression index — `LOWER(email)`,
    /// `COALESCE(scope_kind, '')`), deserialized from the spec once at registration
    /// and evaluated per row, mirroring the real storage's key derivation. `None`
    /// for a plain index.
    key_exprs: Option<Vec<Expr>>,
    /// Partial-index predicate (`None` for a full index), deserialized from the
    /// spec's IR once at registration. The oracle filters scan rows by this so a
    /// non-satisfying row is never a probe match.
    predicate: Option<Expr>,
    /// `NULLS NOT DISTINCT`: NULL keys are probed (two NULLs collide) when true.
    nulls_not_distinct: bool,
    /// The indexed table's column names in row order (predicate-evaluator binding).
    column_names: Vec<String>,
    /// The parallel declared column types (see `IndexSpec::column_types`) — the
    /// oracle must evaluate a stored expression with the same type scope the real
    /// index does, or a stringified `timestamptz` would differ between them.
    column_types: Vec<TypeOid>,
    /// The parallel enum member orders (see `IndexSpec::column_enum_orders`) — the
    /// same reason: the oracle must decide a partial index's membership by the order
    /// the real index maintenance decides it by.
    column_enum_orders: Vec<Option<Vec<String>>>,
}

/// Simple in-memory storage for testing.
#[derive(Debug, Default)]
pub struct MemStorage {
    tables: HashMap<String, Vec<(RowId, Vec<Value>)>>,
    next_id: RowId,
    indexes: HashMap<u32, MemIndexDef>,
    /// How many times [`Storage::scan`] has run, over any table — what a cost guard reads
    /// to tell a statement that read a table whole from one that probed an index.
    scans: AtomicUsize,
}

impl MemStorage {
    /// How many full-table scans ([`Storage::scan`]) this store has served. The oracle's
    /// own index probes filter its rows directly and are not counted.
    pub fn scan_count(&self) -> usize {
        self.scans.load(Ordering::Relaxed)
    }

    /// Row IDs in `table` whose indexed column(s) equal `values`, via a
    /// scan+filter (the source of truth the real index probe must match).
    pub fn scan_filter_ids(&self, table: &str, key_cols: &[usize], values: &[Value]) -> Vec<RowId> {
        let mut ids: Vec<RowId> = self
            .tables
            .get(table)
            .map(|rows| {
                rows.iter()
                    .filter(|(_, row)| {
                        key_cols
                            .iter()
                            .zip(values)
                            .all(|(&col, v)| row.get(col) == Some(v))
                    })
                    .map(|(id, _)| *id)
                    .collect()
            })
            .unwrap_or_default();
        ids.sort_unstable();
        ids
    }

    /// Row IDs the index `def` would hold for the key `values` — the oracle the
    /// real index probe must match. Each row's key tuple is computed the way the
    /// real storage computes it: evaluating the key-part expressions for an
    /// expression index, else reading the bare-column positions. For a partial
    /// index, only rows satisfying the predicate are candidates, so the oracle never
    /// reports a non-indexed row as a uniqueness conflict.
    fn index_filter_ids(&self, def: &MemIndexDef, values: &[Value]) -> Vec<RowId> {
        let Some(rows) = self.tables.get(&def.table) else {
            return Vec::new();
        };
        let col_names: Vec<&str> = def.column_names.iter().map(String::as_str).collect();
        let col_types: &[TypeOid] = &def.column_types;
        // The same member orders the real index maintenance evaluates with, or the
        // oracle would decide a partial index's membership by a different rule than the
        // implementation it is the oracle for.
        let col_enums: Vec<Option<&[String]>> = def
            .column_enum_orders
            .iter()
            .map(|o| o.as_deref())
            .collect();
        let mut ids: Vec<RowId> = rows
            .iter()
            .filter(|(_, row)| {
                // Partial index: a non-satisfying row is not a candidate.
                if let Some(pred) = &def.predicate
                    && !matches!(
                        pglet_sql::executor::eval_expr(
                            pred, &col_names, col_types, &col_enums, row
                        ),
                        Ok(Value::Bool(true))
                    )
                {
                    return false;
                }
                // The row's key tuple: expression parts (evaluated) or bare columns.
                let row_key: Vec<Value> = def.key_exprs.as_ref().map_or_else(
                    || {
                        def.key_cols
                            .iter()
                            .map(|&c| row.get(c).cloned().unwrap_or(Value::Null))
                            .collect()
                    },
                    |exprs| {
                        exprs
                            .iter()
                            .map(|e| {
                                pglet_sql::executor::eval_expr(
                                    e, &col_names, col_types, &col_enums, row,
                                )
                                .unwrap_or(Value::Null)
                            })
                            .collect()
                    },
                );
                row_key.len() == values.len() && row_key.iter().zip(values).all(|(a, b)| a == b)
            })
            .map(|(id, _)| *id)
            .collect();
        ids.sort_unstable();
        ids
    }
}

impl Storage for MemStorage {
    fn create_table(&mut self, name: &str) -> Result<()> {
        self.tables.insert(name.to_string(), Vec::new());
        Ok(())
    }

    fn drop_table(&mut self, name: &str) -> Result<()> {
        self.tables.remove(name);
        Ok(())
    }

    fn insert(&mut self, table: &str, row: Vec<Value>) -> Result<RowId> {
        let id = self.next_id;
        self.next_id += 1;
        if let Some(rows) = self.tables.get_mut(table) {
            rows.push((id, row));
        }
        Ok(id)
    }

    fn scan(&self, table: &str) -> Result<Vec<(RowId, Vec<Value>)>> {
        self.scans.fetch_add(1, Ordering::Relaxed);
        Ok(self.tables.get(table).cloned().unwrap_or_default())
    }

    fn get(&self, table: &str, id: RowId) -> Result<Option<Vec<Value>>> {
        if let Some(rows) = self.tables.get(table) {
            for (row_id, row) in rows {
                if *row_id == id {
                    return Ok(Some(row.clone()));
                }
            }
        }
        Ok(None)
    }

    fn delete(&mut self, table: &str, id: RowId) -> Result<Option<Vec<Value>>> {
        if let Some(rows) = self.tables.get_mut(table)
            && let Some(pos) = rows.iter().position(|(row_id, _)| *row_id == id)
        {
            let (_, row) = rows.remove(pos);
            return Ok(Some(row));
        }
        Ok(None)
    }

    fn update(
        &mut self,
        table: &str,
        id: RowId,
        new_row: Vec<Value>,
    ) -> Result<Option<Vec<Value>>> {
        if let Some(rows) = self.tables.get_mut(table) {
            for (row_id, row) in rows.iter_mut() {
                if *row_id == id {
                    let old = std::mem::replace(row, new_row);
                    return Ok(Some(old));
                }
            }
        }
        Ok(None)
    }

    fn add_column(&mut self, table: &str, default: Value) -> Result<()> {
        if let Some(rows) = self.tables.get_mut(table) {
            for (_, row) in rows.iter_mut() {
                row.push(default.clone());
            }
        }
        Ok(())
    }

    fn drop_column(&mut self, table: &str, column_index: usize) -> Result<()> {
        if let Some(rows) = self.tables.get_mut(table) {
            for (_, row) in rows.iter_mut() {
                if column_index < row.len() {
                    row.remove(column_index);
                }
            }
        }
        Ok(())
    }

    fn create_index(&mut self, spec: pglet_sql::executor::IndexSpec) -> Result<()> {
        // The def is enough since this store probes by scan+filter (it is the
        // oracle the real index must match), so back-fill and per-write
        // maintenance are implicit. Supports composite key columns, the partial
        // predicate, and NULLS NOT DISTINCT (applied in `index_lookup`).
        //
        // `Storage::create_index`'s all-or-nothing contract holds here for want of a
        // failure to violate it with: there is no back-fill, this returns `Ok`
        // unconditionally, and the two IR reads panic rather than erroring — before the
        // insert, so even that leaves the registry as it found it.
        self.indexes.insert(
            spec.index_id,
            MemIndexDef {
                table: spec.table,
                key_cols: spec.key_col_positions,
                // An expression index's key parts (`key_part_ir`), deserialized once.
                key_exprs: spec.key_part_ir.as_ref().map(|parts| {
                    parts
                        .iter()
                        .map(|b| deserialize_expr(b).expect("key part deserializes"))
                        .collect()
                }),
                // Deserialize the partial-index predicate IR once at registration
                // (the bytes come straight from the catalog `Expr`).
                predicate: spec
                    .predicate_ir
                    .as_deref()
                    .map(|b| deserialize_expr(b).expect("predicate deserializes")),
                nulls_not_distinct: spec.nulls_not_distinct,
                column_names: spec.column_names,
                column_types: spec.column_types,
                column_enum_orders: spec.column_enum_orders,
            },
        );
        Ok(())
    }

    fn reattach_index(&mut self, spec: pglet_sql::executor::IndexSpec) -> Result<()> {
        self.create_index(spec)
    }

    fn rebuild_index(&mut self, spec: pglet_sql::executor::IndexSpec) -> Result<()> {
        // `Storage::rebuild_index` is **all-or-nothing**, and this store's defs carry no
        // entries — the module doc's scan-and-filter rule: a probe reads the live rows.
        // So replacing the def *is* the whole rebuild, and `create_index`'s `insert`
        // overwrites in place. Removing the old def first is the shape that made a
        // failing rebuild destructive on `MemoryStorage`, and it is the shape this impl
        // had.
        //
        // Two things made that shape survivable, and "`create_index` returns `Ok`" is
        // only the first. It can still **panic**: its `MemIndexDef` literal carries two
        // `deserialize_expr(…).expect(…)` calls, and the literal is an argument to
        // `self.indexes.insert(…)`, so a panic there happens after the old def was
        // removed and before the new one lands. Keeping the two in one call is
        // panic-safe as well as error-safe, and both properties are today's body's
        // rather than the contract's premise.
        //
        // Neither is exercised: measured by replacing this body with a `panic!`, the
        // whole `cargo test --workspace` stays green, none of it reaching
        // `MemStorage::rebuild_index` at all. So this is a contract kept by reading, not
        // by a guard, and adding one would mean building a `TestDb` shape that issues
        // `CREATE TYPE`/`DROP TYPE` over a table this oracle already indexes.
        self.create_index(spec)
    }

    fn drop_index(&mut self, index_id: u32) -> Result<()> {
        self.indexes.remove(&index_id);
        Ok(())
    }

    fn index_lookup(&self, index_id: u32, values: &[Value]) -> Result<Vec<RowId>> {
        let Some(def) = self.indexes.get(&index_id) else {
            return Ok(Vec::new());
        };
        // NULL key columns: no match under default NULLS DISTINCT; probed under NND.
        if !def.nulls_not_distinct && values.iter().any(|v| matches!(v, Value::Null)) {
            return Ok(Vec::new());
        }
        Ok(self.index_filter_ids(def, values))
    }

    fn gin_lookup(&self, index_id: u32, keys: &[Value]) -> Result<Vec<RowId>> {
        // The scan+filter oracle: a row is a candidate when it's in the (partial)
        // index AND its single GIN key value's decomposition holds every query key
        // (`gin_contains_keys` — the same lossy superset the real posting-intersection
        // returns, before the executor's `@>` recheck). Empty `keys` never reaches
        // here (the caller full-scans `x @> {}`); return nothing if it does.
        let Some(def) = self.indexes.get(&index_id) else {
            return Ok(Vec::new());
        };
        if keys.is_empty() {
            return Ok(Vec::new());
        }
        let col = def.key_cols.first().copied().unwrap_or(0);
        let predicate = def.predicate.as_ref();
        let col_names: Vec<&str> = def.column_names.iter().map(String::as_str).collect();
        let col_types: &[TypeOid] = &def.column_types;
        let col_enums: Vec<Option<&[String]>> = def
            .column_enum_orders
            .iter()
            .map(|o| o.as_deref())
            .collect();
        let mut ids: Vec<RowId> = self
            .tables
            .get(&def.table)
            .map(|rows| {
                rows.iter()
                    .filter(|(_, row)| {
                        if let Some(pred) = predicate
                            && !matches!(
                                pglet_sql::executor::eval_expr(
                                    pred, &col_names, col_types, &col_enums, row
                                ),
                                Ok(Value::Bool(true))
                            )
                        {
                            return false;
                        }
                        row.get(col)
                            .is_some_and(|v| pglet_sql::gin::gin_contains_keys(v, keys))
                    })
                    .map(|(id, _)| *id)
                    .collect()
            })
            .unwrap_or_default();
        ids.sort_unstable();
        Ok(ids)
    }

    fn index_range_lookup(
        &self,
        index_id: u32,
        lower: Option<(Value, bool)>,
        upper: Option<(Value, bool)>,
        ascending: bool,
        limit: Option<usize>,
    ) -> Result<Vec<RowId>> {
        // Single-column oracle: gather (key bytes, row_id) for the index's rows
        // (predicate-satisfying, non-NULL key), keep those in range, sort by the
        // order-preserving key bytes, apply direction + limit. The order-preserving
        // `encode_index_key` bytes give the same order the real index uses.
        let Some(def) = self.indexes.get(&index_id) else {
            return Ok(Vec::new());
        };
        let col = def.key_cols.first().copied().unwrap_or(0);
        // A NULL bound is meaningless for a range; treat as unbounded that side.
        let enc = |b: &Option<(Value, bool)>| -> Option<(Vec<u8>, bool)> {
            b.as_ref()
                .filter(|(v, _)| !matches!(v, Value::Null))
                .map(|(v, incl)| (v.encode_index_key().expect("indexable bound"), *incl))
        };
        let lo = enc(&lower);
        let hi = enc(&upper);

        // Candidate ids (predicate-filtered) — reuse the equality oracle's predicate
        // path by scanning all rows of the table and applying the predicate.
        let predicate = def.predicate.as_ref();
        let col_names: Vec<&str> = def.column_names.iter().map(String::as_str).collect();
        let col_types: &[TypeOid] = &def.column_types;
        let col_enums: Vec<Option<&[String]>> = def
            .column_enum_orders
            .iter()
            .map(|o| o.as_deref())
            .collect();

        let mut hits: Vec<(Vec<u8>, RowId)> = Vec::new();
        for (id, row) in self.tables.get(&def.table).cloned().unwrap_or_default() {
            // Partial-index predicate gate.
            if let Some(pred) = predicate
                && !matches!(
                    pglet_sql::executor::eval_expr(pred, &col_names, col_types, &col_enums, &row),
                    Ok(Value::Bool(true))
                )
            {
                continue;
            }
            // NULL keys never fall in a value range.
            let Some(v) = row.get(col) else { continue };
            if matches!(v, Value::Null) {
                continue;
            }
            let key = v.encode_index_key().expect("indexable key");
            let lower_ok = lo.as_ref().is_none_or(|(b, incl)| {
                if *incl {
                    key.as_slice() >= b.as_slice()
                } else {
                    key.as_slice() > b.as_slice()
                }
            });
            let upper_ok = hi.as_ref().is_none_or(|(b, incl)| {
                if *incl {
                    key.as_slice() <= b.as_slice()
                } else {
                    key.as_slice() < b.as_slice()
                }
            });
            if lower_ok && upper_ok {
                hits.push((key, id));
            }
        }
        // Order by (key, row_id) — index-key order.
        hits.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
        let mut ids: Vec<RowId> = hits.into_iter().map(|(_, id)| id).collect();
        if !ascending {
            ids.reverse();
        }
        if let Some(n) = limit {
            ids.truncate(n);
        }
        Ok(ids)
    }
    // The oracle double models the storage *contract*, not a transaction engine —
    // it has no `begin`/`commit` either. A savepoint over it would be a no-op that
    // reads as support, so it refuses.
    fn savepoint(&mut self) -> Result<()> {
        Err(pglet_sql::Error::NotImplemented(
            "the MemStorage oracle models no transactions".into(),
        ))
    }
    fn rollback_to_savepoint(&mut self) -> Result<()> {
        Err(pglet_sql::Error::NotImplemented(
            "the MemStorage oracle models no transactions".into(),
        ))
    }
    fn release_savepoint(&mut self) -> Result<()> {
        Err(pglet_sql::Error::NotImplemented(
            "the MemStorage oracle models no transactions".into(),
        ))
    }
}

/// Test database with schema and storage.
#[derive(Debug)]
pub struct TestDb {
    pub schema: Schema,
    pub storage: MemStorage,
    /// Per-session sequence-function state (`currval` cache) — a `TestDb` is one
    /// session, so it persists across `run` calls.
    pub sequence_session: SequenceSession,
}

impl Default for TestDb {
    fn default() -> Self {
        Self::new()
    }
}

impl TestDb {
    pub fn new() -> Self {
        Self {
            schema: Schema::new(),
            storage: MemStorage::default(),
            sequence_session: SequenceSession::new(),
        }
    }

    pub fn run(&mut self, sql: &str) -> QueryResult {
        let mut plans = parse(sql, &self.schema).expect("parse failed");
        assert_eq!(plans.len(), 1, "expected single statement");
        let p = plans.remove(0);
        execute_with_storage(
            &p,
            &mut self.schema,
            &mut self.storage,
            EvalEnv::EMPTY,
            &mut self.sequence_session,
        )
        .expect("execute failed")
    }

    pub fn try_run(&mut self, sql: &str) -> std::result::Result<QueryResult, pglet_sql::Error> {
        let mut plans = parse(sql, &self.schema)?;
        assert_eq!(plans.len(), 1, "expected single statement");
        let p = plans.remove(0);
        execute_with_storage(
            &p,
            &mut self.schema,
            &mut self.storage,
            EvalEnv::EMPTY,
            &mut self.sequence_session,
        )
    }
}

/// Set up a table and insert data.
pub fn setup_users(db: &mut TestDb) {
    db.run("CREATE TABLE users (id INT, name TEXT, active BOOL)");
    db.run("INSERT INTO users VALUES (1, 'Alice', true), (2, 'Bob', false), (3, 'Carol', true)");
}

/// Create the spine-style `cell_visibility` enum + a table that uses it.
pub fn setup_enum(db: &mut TestDb) {
    db.run("CREATE TYPE cell_visibility AS ENUM ('private', 'public')");
    // `id` is a PRIMARY KEY so `ON CONFLICT (id)` has a real arbiter unique index
    // (Postgres requires one — an unbacked target is a 42P10 error).
    db.run(
        "CREATE TABLE cell (id INT PRIMARY KEY, visibility cell_visibility NOT NULL DEFAULT 'private')",
    );
}

/// A `text[]` value with the given members.
pub fn text_array(members: &[&str]) -> Value {
    Value::Array(ArrayValue::new(
        TypeOid::Text,
        members
            .iter()
            .map(|s| Value::Text((*s).to_string()))
            .collect(),
    ))
}
