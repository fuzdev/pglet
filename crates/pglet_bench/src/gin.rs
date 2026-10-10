//! GIN containment latency (the "gin" scenario).
//!
//! Single-threaded `refs @> ARRAY['tok']` latency over a prepopulated `text[]`
//! table — GIN-accelerated vs full-scan, both within pglet (the speedup a GIN index
//! buys). The SAME data and probes run once with NO index (every probe
//! is a sequential scan + `@>` recheck) and once with a `USING gin(refs)` index (a
//! posting-list intersection + recheck of a small candidate set). SQLite has no
//! `@>`/GIN, so this is pglet-vs-pglet, not cross-engine.

use std::time::Instant;

use pglet::Database;

use crate::common::{
    QUERY_SAMPLES, QUERY_WARMUP, QueryRow, bench_results_path, open_pglet, prepopulate_rows,
    pseudo_a, summarize_query, write_results,
};

/// Distinct array tokens (`t0..t{GIN_VOCAB-1}`) — the containment vocabulary.
const GIN_VOCAB: u64 = 100;
/// Tokens per row, so `@> ARRAY['tK']` matches ~`rows * GIN_REFS_PER_ROW / GIN_VOCAB`.
const GIN_REFS_PER_ROW: u64 = 3;

/// Deterministic small `text[]` literal for row `id`: `GIN_REFS_PER_ROW` tokens
/// drawn pseudo-randomly from the `GIN_VOCAB`-token vocabulary.
fn gin_insert_sql(id: u64) -> String {
    let mut toks = Vec::with_capacity(GIN_REFS_PER_ROW as usize);
    for k in 0..GIN_REFS_PER_ROW {
        toks.push(format!(
            "t{}",
            pseudo_a(id.wrapping_add(k.wrapping_mul(0x9E37))) % GIN_VOCAB
        ));
    }
    format!(
        "INSERT INTO gincell (id, refs) VALUES ({id}, '{{{}}}')",
        toks.join(",")
    )
}

/// Per-sample `@>` probe SQL, varying the probed token across the vocabulary.
fn gin_query_sqls() -> Vec<String> {
    let n = QUERY_WARMUP + QUERY_SAMPLES;
    (0..n)
        .map(|i| {
            format!(
                "SELECT id FROM gincell WHERE refs @> ARRAY['t{}']",
                (i as u64) % GIN_VOCAB
            )
        })
        .collect()
}

/// Measure the prebuilt probe SQLs against `db`, returning (latencies µs, rows_out).
fn gin_measure(db: &mut Database, sqls: &[String]) -> (Vec<f64>, usize) {
    for s in sqls.iter().take(QUERY_WARMUP) {
        std::hint::black_box(&db.execute(s).expect("warmup"));
    }
    let mut lat = Vec::with_capacity(QUERY_SAMPLES);
    let mut rows_out = 0usize;
    for s in sqls.iter().skip(QUERY_WARMUP) {
        let start = Instant::now();
        let r = db.execute(s).expect("gin query");
        lat.push(start.elapsed().as_nanos() as f64 / 1000.0);
        rows_out = r.iter().map(|q| q.rows.len()).sum();
        std::hint::black_box(&r);
    }
    (lat, rows_out)
}

fn bench_gin(conn_id: i32, rows: u64) -> Vec<QueryRow> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("gin.pglet");
    let (mut db, _counters) = open_pglet(&path);
    db.execute("CREATE TABLE gincell (id BIGINT, refs TEXT[])")
        .expect("create table");

    db.begin(conn_id).expect("begin prepopulate");
    for id in 0..rows {
        db.execute_as(&gin_insert_sql(id), conn_id)
            .expect("prepopulate insert");
    }
    db.commit(conn_id).expect("commit prepopulate");

    let sqls = gin_query_sqls();
    // Baseline: NO index → every probe is a sequential scan + `@>` recheck.
    let (scan_lat, scan_rows) = gin_measure(&mut db, &sqls);
    // Accelerated: build the GIN index, re-measure the same probes.
    db.execute("CREATE INDEX gincell_refs_gin ON gincell USING gin(refs)")
        .expect("create gin index");
    let (gin_lat, gin_rows) = gin_measure(&mut db, &sqls);

    vec![
        summarize_query("pglet_scan", "array_contains", scan_lat, scan_rows),
        summarize_query("pglet_gin", "array_contains", gin_lat, gin_rows),
    ]
}

pub fn run_gin(update: bool) {
    use core::fmt::Write as _;
    let rows = prepopulate_rows();
    eprintln!(
        "pglet_bench: GIN containment latency (native L1, read-only). \
prepopulate={rows} rows; vocab={GIN_VOCAB} tokens, {GIN_REFS_PER_ROW}/row"
    );
    eprintln!("  pglet scan vs gin ...");
    let results = bench_gin(1, rows);

    println!("\n=== GIN `refs @> ARRAY[tok]` latency (native L1, scan vs gin) ===");
    println!(
        "prepopulate={rows} rows; row=(id BIGINT, refs TEXT[{GIN_REFS_PER_ROW}]); \
vocab={GIN_VOCAB}; warmup={QUERY_WARMUP} samples={QUERY_SAMPLES}"
    );
    println!(
        "{:<8} {:>12} {:>12} {:>8} {:>12}",
        "engine", "p50_us", "p95_us", "cv", "rows_out"
    );
    println!("{}", "-".repeat(56));
    for r in &results {
        println!(
            "{:<8} {:>12.2} {:>12.2} {:>8.3} {:>12}",
            r.engine.strip_prefix("pglet_").unwrap_or(&r.engine),
            r.p50_us,
            r.p95_us,
            r.cv,
            r.rows_out
        );
    }
    println!("{}", "-".repeat(56));

    let p50_of = |engine: &str| -> f64 {
        results
            .iter()
            .find(|r| r.engine == engine)
            .map_or(0.0, |r| r.p50_us)
    };
    let (scan, gin) = (p50_of("pglet_scan"), p50_of("pglet_gin"));
    let speedup = if gin > 0.0 { scan / gin } else { 0.0 };
    println!(
        "\nscan p50 {scan:.2}us / gin p50 {gin:.2}us  →  {speedup:.1}x speedup (higher is better)"
    );

    let mut json = String::new();
    json.push_str("{\n  \"meta\": {\n");
    json.push_str("    \"benchmark\": \"gin_containment_latency\",\n");
    json.push_str("    \"lane\": \"native_inprocess\",\n");
    json.push_str("    \"layer\": \"L1\",\n");
    let _ = writeln!(json, "    \"prepopulate_rows\": {rows},");
    let _ = writeln!(
        json,
        "    \"vocab_tokens\": {GIN_VOCAB},\n    \"refs_per_row\": {GIN_REFS_PER_ROW},"
    );
    let _ = writeln!(
        json,
        "    \"warmup\": {QUERY_WARMUP},\n    \"samples\": {QUERY_SAMPLES},"
    );
    let _ = writeln!(json, "    \"scan_over_gin_p50_speedup\": {speedup:.3},");
    json.push_str("    \"units\": \"microseconds\",\n");
    json.push_str("    \"higher_or_lower_is_better\": \"lower\"\n  },\n");
    json.push_str("  \"rows\": [\n");
    for (i, r) in results.iter().enumerate() {
        json.push_str("    ");
        json.push_str(&r.to_json());
        json.push_str(if i + 1 < results.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ]\n}\n");

    write_results(&bench_results_path("gin_results.json"), &json, update);

    println!("\n=== JSON ===");
    println!("{json}");
}
