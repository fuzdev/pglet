//! Query/scan latency (the "query" scenario).
//!
//! Single-threaded read-path latency for pglet vs SQLite (rusqlite), over a
//! prepopulated table with NO secondary index — so every engine answers each
//! query by sequential scan, keeping it apples-to-apples: point lookup = scan +
//! equality filter (one row out), full scan = scan + materialize all rows,
//! aggregate = scan + count/sum. SQL strings are pre-generated outside the timed
//! region, and each timed iteration re-parses+plans+executes (rusqlite `prepare`
//! per iteration, matching pglet's parse-per-`execute`) so neither side gets a
//! free cached plan. Both sides fully materialize the result rows.

use std::time::Instant;

use crate::common::{
    CREATE_SQL, QUERY_SAMPLES, QUERY_WARMUP, QueryRow, SQLITE_CACHE_SIZE, SQLITE_PAGE_SIZE, TEXT_B,
    bench_results_path, insert_sql, open_pglet, prepopulate_rows, pseudo_a, summarize_query,
    write_results,
};

const QUERY_SCENARIOS: &[&str] = &["point_lookup", "full_scan", "aggregate"];

/// Pre-generate the per-sample SQL (warmup + measured) for a scenario, so the
/// `format!` cost stays out of the timed region. Point lookup varies the key per
/// sample; scan/aggregate repeat the same statement. `sum(id)` (not `sum(a)`)
/// because `id` is `0..rows` — its sum fits `i64`, so neither engine overflows.
fn query_sqls(scenario: &str, rows: u64) -> Vec<String> {
    let n = QUERY_WARMUP + QUERY_SAMPLES;
    match scenario {
        "point_lookup" => (0..n)
            .map(|i| {
                format!(
                    "SELECT id, a, b FROM bench WHERE id = {}",
                    pseudo_a(i as u64) % rows
                )
            })
            .collect(),
        "full_scan" => vec!["SELECT id, a, b FROM bench".to_string(); n],
        "aggregate" => vec!["SELECT count(*), sum(id) FROM bench".to_string(); n],
        _ => unreachable!("unknown query scenario: {scenario}"),
    }
}

fn bench_query_pglet(conn_id: i32, rows: u64) -> Vec<QueryRow> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("query.pglet");
    let (mut db, _counters) = open_pglet(&path);
    db.execute(CREATE_SQL).expect("create table");

    db.begin(conn_id).expect("begin prepopulate");
    for id in 0..rows {
        db.execute_as(&insert_sql(id, pseudo_a(id)), conn_id)
            .expect("prepopulate insert");
    }
    db.commit(conn_id).expect("commit prepopulate");

    let mut out = Vec::new();
    for &scenario in QUERY_SCENARIOS {
        let sqls = query_sqls(scenario, rows);
        for s in sqls.iter().take(QUERY_WARMUP) {
            std::hint::black_box(&db.execute(s).expect("warmup query"));
        }
        let mut lat = Vec::with_capacity(QUERY_SAMPLES);
        let mut rows_out = 0usize;
        for s in sqls.iter().skip(QUERY_WARMUP) {
            let start = Instant::now();
            let r = db.execute(s).expect("query");
            lat.push(start.elapsed().as_nanos() as f64 / 1000.0);
            rows_out = r.iter().map(|q| q.rows.len()).sum();
            std::hint::black_box(&r);
        }
        out.push(summarize_query("pglet", scenario, lat, rows_out));
    }
    out
}

/// Prepare + fully consume a SQLite query (re-parsing each call). Returns the
/// materialized row count. `is_agg` reads the 2-column `(count, sum)` shape;
/// otherwise the 3-column `(id, a, b)` shape — matching pglet's materialized rows.
fn consume_sqlite(conn: &rusqlite::Connection, sql: &str, is_agg: bool) -> usize {
    let mut stmt = conn.prepare(sql).expect("prepare");
    let mut rows = stmt.query([]).expect("query");
    let mut n = 0usize;
    while let Some(row) = rows.next().expect("row") {
        if is_agg {
            let count: i64 = row.get(0).expect("count");
            let sum: Option<i64> = row.get(1).expect("sum");
            std::hint::black_box((count, sum));
        } else {
            let id: i64 = row.get(0).expect("id");
            let a: i64 = row.get(1).expect("a");
            let b: String = row.get(2).expect("b");
            std::hint::black_box((id, a, b));
        }
        n += 1;
    }
    n
}

fn bench_query_sqlite(rows: u64) -> Vec<QueryRow> {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("query.sqlite");
    let conn = rusqlite::Connection::open(&path).expect("open sqlite");
    conn.pragma_update(None, "page_size", SQLITE_PAGE_SIZE)
        .expect("page_size");
    conn.pragma_update(None, "cache_size", SQLITE_CACHE_SIZE)
        .expect("cache_size");
    conn.execute_batch(CREATE_SQL).expect("create table");

    conn.execute_batch("BEGIN").expect("begin prepopulate");
    {
        let mut stmt = conn
            .prepare("INSERT INTO bench VALUES (?1, ?2, ?3)")
            .expect("prepare prepopulate");
        for id in 0..rows {
            stmt.execute(rusqlite::params![id as i64, pseudo_a(id) as i64, TEXT_B])
                .expect("prepopulate insert");
        }
    }
    conn.execute_batch("COMMIT").expect("commit prepopulate");

    let mut out = Vec::new();
    for &scenario in QUERY_SCENARIOS {
        let is_agg = scenario == "aggregate";
        let sqls = query_sqls(scenario, rows);
        for s in sqls.iter().take(QUERY_WARMUP) {
            std::hint::black_box(consume_sqlite(&conn, s, is_agg));
        }
        let mut lat = Vec::with_capacity(QUERY_SAMPLES);
        let mut rows_out = 0usize;
        for s in sqls.iter().skip(QUERY_WARMUP) {
            let start = Instant::now();
            rows_out = consume_sqlite(&conn, s, is_agg);
            lat.push(start.elapsed().as_nanos() as f64 / 1000.0);
        }
        out.push(summarize_query("sqlite", scenario, lat, rows_out));
    }
    out
}

/// The `query` scenario: single-threaded read-path latency, pglet vs SQLite.
pub fn run_query(update: bool) {
    use core::fmt::Write as _;
    let rows = prepopulate_rows();
    eprintln!(
        "pglet_bench: query/scan latency (native L1, read-only). \
prepopulate={rows} rows; scenarios={QUERY_SCENARIOS:?}"
    );

    let mut results: Vec<QueryRow> = Vec::new();
    eprintln!("  pglet query ...");
    results.extend(bench_query_pglet(1, rows));
    eprintln!("  sqlite query ...");
    results.extend(bench_query_sqlite(rows));

    println!("\n=== query/scan latency (native L1, read-only, no secondary index) ===");
    println!(
        "prepopulate={rows} rows; row=(id BIGINT, a BIGINT, b TEXT[{}]); \
SQLite page_size={SQLITE_PAGE_SIZE} cache_size={SQLITE_CACHE_SIZE}; \
warmup={QUERY_WARMUP} samples={QUERY_SAMPLES}",
        TEXT_B.len()
    );
    println!(
        "{:<14} {:<8} {:>12} {:>12} {:>8} {:>12}",
        "scenario", "engine", "p50_us", "p95_us", "cv", "rows_out"
    );
    println!("{}", "-".repeat(70));
    for &scenario in QUERY_SCENARIOS {
        for r in results.iter().filter(|r| r.scenario == scenario) {
            println!(
                "{:<14} {:<8} {:>12.2} {:>12.2} {:>8.3} {:>12}",
                r.scenario, r.engine, r.p50_us, r.p95_us, r.cv, r.rows_out
            );
        }
        println!("{}", "-".repeat(70));
    }

    println!("\n=== pglet / SQLite p50 ratio (lower pglet is better) ===");
    println!(
        "{:<14} {:>14} {:>16} {:>10}",
        "scenario", "pglet_p50_us", "sqlite_p50_us", "ratio"
    );
    let p50_of = |engine: &str, scenario: &str| -> f64 {
        results
            .iter()
            .find(|r| r.engine == engine && r.scenario == scenario)
            .map_or(0.0, |r| r.p50_us)
    };
    for &scenario in QUERY_SCENARIOS {
        let p = p50_of("pglet", scenario);
        let s = p50_of("sqlite", scenario);
        let ratio = if s > 0.0 { p / s } else { 0.0 };
        println!("{scenario:<14} {p:>14.2} {s:>16.2} {ratio:>10.3}");
    }

    let mut json = String::new();
    json.push_str("{\n  \"meta\": {\n");
    json.push_str("    \"benchmark\": \"query_scan_latency\",\n");
    json.push_str("    \"lane\": \"native_inprocess\",\n");
    json.push_str("    \"layer\": \"L1\",\n");
    let _ = writeln!(json, "    \"prepopulate_rows\": {rows},");
    json.push_str("    \"index\": \"none (sequential scan for every scenario)\",\n");
    let _ = writeln!(
        json,
        "    \"warmup\": {QUERY_WARMUP},\n    \"samples\": {QUERY_SAMPLES},"
    );
    json.push_str("    \"units\": \"microseconds\",\n");
    json.push_str("    \"higher_or_lower_is_better\": \"lower\"\n  },\n");
    json.push_str("  \"rows\": [\n");
    for (i, r) in results.iter().enumerate() {
        json.push_str("    ");
        json.push_str(&r.to_json());
        json.push_str(if i + 1 < results.len() { ",\n" } else { "\n" });
    }
    json.push_str("  ]\n}\n");

    write_results(&bench_results_path("query_results.json"), &json, update);

    println!("\n=== JSON ===");
    println!("{json}");
}
