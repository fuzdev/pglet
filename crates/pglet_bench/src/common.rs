//! Helpers shared across two or more bench scenarios: the prepopulate knob, the
//! pglet `Database` builder over the double-meta store, the shared insert/SQLite
//! constants, and the `QueryRow` summary shape used by both `query` and `gin`.

use std::path::{Path, PathBuf};

use pglet::{Database, StorageBackend};
use pglet_sql::catalog::Schema;
use pglet_storage::btree::{
    BtreeStorageWrapper, CountingBlockIo, CountingHandle, CowBtree, DoubleMetaStore, FileBlockIo,
};

use crate::stats::{cv, percentile_sorted};

/// Rows pre-loaded (in one explicit transaction) before measuring, so the
/// B+tree has realistic depth. Setup, not measured. Overridable via
/// `PGLET_BENCH_PREPOPULATE` (e.g. a small value for a fast correctness smoke).
pub fn prepopulate_rows() -> u64 {
    std::env::var("PGLET_BENCH_PREPOPULATE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(100_000)
}

/// The fixed-length text payload for column `b` (~32 chars), so row size is
/// realistic rather than a few bytes.
pub const TEXT_B: &str = "abcdefghijklmnopqrstuvwxyz012345"; // 32 chars

/// SQLite pragmas pinned for every config.
pub const SQLITE_PAGE_SIZE: i64 = 4096;
pub const SQLITE_CACHE_SIZE: i64 = 2000; // pages; matches pglet's default block cache.

/// Build a `Database` over `DoubleMetaStore` over `CountingBlockIo` over
/// `FileBlockIo` at `path`. Returns the database plus a cloned counter handle
/// (cloned *before* the counter is boxed into the store).
pub fn open_pglet(path: &Path) -> (Database, CountingHandle) {
    let file_io = FileBlockIo::open(path).expect("open file block io");
    let counting = CountingBlockIo::new(file_io);
    let handle = counting.handle();

    // `PGLET_BENCH_CACHE` overrides the page-cache size (default 2000). Set it to
    // 1 to effectively disable the read cache for a profile-only A/B split.
    let cache_size: usize = std::env::var("PGLET_BENCH_CACHE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(2000);
    let (store, metadata) = DoubleMetaStore::open_with_cache_size(Box::new(counting), cache_size)
        .expect("open double-meta store");
    let btree = CowBtree::with_store(Box::new(store), metadata.root, metadata.version, 100);
    // Recover the table directory, row-id counters, and catalog from the tree
    // (PGM5 system entries), not the slot.
    let (wrapper, schema_bytes) =
        BtreeStorageWrapper::recover(std::sync::Arc::new(btree), metadata.next_table_id)
            .expect("recover catalog from tree");
    let schema = if schema_bytes.is_empty() {
        Schema::new()
    } else {
        postcard::from_bytes(&schema_bytes).expect("deserialize schema")
    };
    let db = Database::with_storage(StorageBackend::Btree(wrapper), schema);
    (db, handle)
}

pub const CREATE_SQL: &str = "CREATE TABLE bench (id BIGINT, a BIGINT, b TEXT)";

/// Rows per prepopulate commit. Prepopulate loads in bounded chunks rather than
/// one giant transaction so setup doesn't pay the apply-loop's within-batch
/// re-descent/clone cost — which is quadratic in batch size and, on the
/// `Enforcing` schema (three B+trees, a random-order secondary key), makes a
/// single 100k-row batch pathological. Setup isn't measured; the chunk size only
/// bounds its cost.
pub const PREPOPULATE_CHUNK: u64 = 2_000;

/// Prepopulate `db` with `rows` rows in `PREPOPULATE_CHUNK`-sized transactions,
/// returning the next unused id. Setup, not measured. `conn_id` routes the
/// transaction; the row shape matches `insert_sql`.
pub fn prepopulate_pglet(db: &mut Database, conn_id: i32, rows: u64) -> u64 {
    let mut next_id = 0u64;
    let mut done = 0u64;
    while done < rows {
        let chunk = PREPOPULATE_CHUNK.min(rows - done);
        db.begin(conn_id).expect("begin prepopulate chunk");
        for _ in 0..chunk {
            let a = pseudo_a(next_id);
            db.execute_as(&insert_sql(next_id, a), conn_id)
                .expect("prepopulate insert");
            next_id += 1;
        }
        db.commit(conn_id).expect("commit prepopulate chunk");
        done += chunk;
    }
    next_id
}

/// Which table shape a write-path bench builds — the schema axis of the honest
/// re-baseline. `IndexFree` is the optimistic ceiling (no uniqueness probe, no
/// index B+tree maintenance per insert); `Enforcing` adds a PRIMARY KEY (an
/// enforced unique index probed on every insert) plus a secondary index — the
/// per-insert cost a real, constraint-enforcing schema actually pays. Both
/// engines build the same DDL per axis, so the pglet/SQLite ratio stays
/// structure-vs-structure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchSchema {
    /// `CREATE TABLE bench (id BIGINT, a BIGINT, b TEXT)` — no PK, no index.
    IndexFree,
    /// The `IndexFree` table with `id BIGINT PRIMARY KEY` plus a secondary index
    /// on `a`, so each insert pays a PK uniqueness probe and maintains two index
    /// B+trees.
    Enforcing,
}

impl BenchSchema {
    /// Short axis label recorded in each result row.
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::IndexFree => "indexfree",
            Self::Enforcing => "enforcing",
        }
    }

    /// The inverse of `label`: map an axis label back to its schema (any
    /// unrecognized label falls back to `IndexFree` — callers only ever pass the
    /// two known labels, e.g. across the `memory` bench's parent→child hop).
    #[must_use]
    pub fn from_label(label: &str) -> Self {
        match label {
            "enforcing" => Self::Enforcing,
            _ => Self::IndexFree,
        }
    }

    /// The setup DDL, in order: one statement for `IndexFree`, the table plus a
    /// secondary `CREATE INDEX` for `Enforcing`. Both engines accept it verbatim
    /// — SQLite gives `id BIGINT PRIMARY KEY` its own unique index too (only the
    /// exact type name `INTEGER` aliases the rowid), so the PK is a real index
    /// probe on both sides, not a free rowid.
    #[must_use]
    pub const fn create_stmts(self) -> &'static [&'static str] {
        match self {
            Self::IndexFree => &[CREATE_SQL],
            Self::Enforcing => &[
                "CREATE TABLE bench (id BIGINT PRIMARY KEY, a BIGINT, b TEXT)",
                "CREATE INDEX bench_a_idx ON bench (a)",
            ],
        }
    }
}

/// The write-path schema axes the commit + attribute sweeps cover: the
/// index-free ceiling and the enforcing (PK + secondary index) shape. Both sweep
/// the same batch sizes and prepopulate — the enforcing per-insert cost is now a
/// small constant factor over index-free (flat across batch size), so it needs no
/// throttling.
pub const BENCH_SCHEMAS: &[BenchSchema] = &[BenchSchema::IndexFree, BenchSchema::Enforcing];

pub fn insert_sql(id: u64, a: u64) -> String {
    // Column `a` is BIGINT (signed i64); mask the (possibly > i64::MAX) `pseudo_a`
    // filler into range so the insert doesn't fail loud (`22003`, the engine is
    // correct-or-loud). `a` is row-width filler never read as a value by any bench
    // (gets/predicates key off `id`), so masking the sign bit is value-neutral.
    let a = a & 0x7FFF_FFFF_FFFF_FFFF;
    format!("INSERT INTO bench VALUES ({id}, {a}, '{TEXT_B}')")
}

/// Cheap deterministic-ish pseudo-random for column `a`.
pub const fn pseudo_a(id: u64) -> u64 {
    // SplitMix64-ish; good enough for "random-ish" non-monotonic values.
    let mut z = id.wrapping_mul(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

pub const QUERY_WARMUP: usize = 5;
pub const QUERY_SAMPLES: usize = 30;

#[derive(Debug, Clone)]
pub struct QueryRow {
    pub engine: String,
    pub scenario: String,
    pub p50_us: f64,
    pub p95_us: f64,
    pub cv: f64,
    /// Rows materialized by the last sample (1 for point lookup, N for scan, 1
    /// for aggregate) — sanity/context, not a timing.
    pub rows_out: usize,
}

impl QueryRow {
    pub fn to_json(&self) -> String {
        format!(
            "{{\"engine\":{:?},\"scenario\":{:?},\"p50_us\":{:.3},\"p95_us\":{:.3},\
\"cv\":{:.4},\"rows_out\":{},\"units\":\"microseconds\",\
\"higher_or_lower_is_better\":\"lower\"}}",
            self.engine, self.scenario, self.p50_us, self.p95_us, self.cv, self.rows_out,
        )
    }
}

/// Sort raw µs latencies and fold them into a `QueryRow`.
pub fn summarize_query(
    engine: &str,
    scenario: &str,
    mut lat: Vec<f64>,
    rows_out: usize,
) -> QueryRow {
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap());
    QueryRow {
        engine: engine.to_string(),
        scenario: scenario.to_string(),
        p50_us: percentile_sorted(&lat, 0.50),
        p95_us: percentile_sorted(&lat, 0.95),
        cv: cv(&lat),
        rows_out,
    }
}

/// Resolve `benches/<filename>` at the workspace root.
///
/// `CARGO_MANIFEST_DIR` is `<root>/crates/pglet_bench`, so the root is two parents
/// up. The single home for the per-scenario `*_results.json` path each bench writes
/// — callers pass the full file name (`parse_results.json`, `phase1_results.json`,
/// …) since the names don't all follow the `<scenario>_results.json` shape.
#[must_use]
pub fn bench_results_path(filename: &str) -> PathBuf {
    let manifest = env!("CARGO_MANIFEST_DIR");
    let root = Path::new(manifest)
        .parent()
        .and_then(Path::parent)
        .unwrap_or_else(|| Path::new("."));
    root.join("benches").join(filename)
}

/// Write `json` to `path` when `update`, else print the not-written hint.
///
/// The shared `--update` write block every bench scenario funnels through (the
/// `size:check`/`:update` idiom): a plain run prints but leaves the committed
/// `benches/*_results.json` untouched; `--update` (re)writes it, creating the
/// parent dir. A write failure warns rather than aborting the bench.
pub fn write_results(path: &Path, json: &str, update: bool) {
    if update {
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match std::fs::write(path, json) {
            Ok(()) => eprintln!("wrote {}", path.display()),
            Err(e) => eprintln!("WARNING: failed to write {}: {e}", path.display()),
        }
    } else {
        eprintln!(
            "(results not written — pass --update to refresh {})",
            path.display()
        );
    }
}
