//! Parse/plan front-end latency (the "parse" scenario).
//!
//! Single-threaded, pglet-only microbench of the SQL front-end — `parse` (text →
//! AST), `plan` (AST → `Plan` against a fixed schema), and the combined
//! parse+plan cold path — over a representative spine-ish query set. No storage,
//! no execution, no SQLite: this isolates the cost the custom-parser swap targets,
//! so it is the **perf baseline** the recorded results track
//! (`benches/parse_results.json`; see `docs/parser.md`).
//!
//! The schema is built once (a handful of `CREATE`s resembling the spine's
//! cell/role-grant tables, plus a GIN index) and reused; each timed iteration
//! re-parses (and re-plans) one statement from cold, so no cached plan leaks in.

use std::time::Instant;

use pglet::Database;

use crate::common::{bench_results_path, write_results};
use crate::stats::{cv, percentile_sorted};

/// Warmup iterations per case (discarded), then measured samples.
const PARSE_WARMUP: usize = 500;
const PARSE_SAMPLES: usize = 5000;

/// DDL run once to give the planner a realistic catalog to resolve against.
const SCHEMA_SQL: &[&str] = &[
    "CREATE TABLE cell (id UUID PRIMARY KEY, name TEXT, data JSONB, refs TEXT[], \
     created TIMESTAMPTZ, deleted_at TIMESTAMPTZ)",
    "CREATE TABLE role_grant (id UUID PRIMARY KEY, actor_id UUID, role TEXT, \
     scope_kind TEXT, scope_id UUID, revoked_at TIMESTAMPTZ)",
    "CREATE INDEX cell_data_gin ON cell USING gin(data)",
    "CREATE INDEX cell_refs_gin ON cell USING gin(refs)",
];

/// A representative statement to measure, with a stable label.
struct ParseCase {
    label: &'static str,
    sql: &'static str,
}

/// The query set — a spread across the shapes the spine actually issues: point
/// reads, JSONB / array containment, INSERT (incl. column-target ON CONFLICT),
/// UPDATE, DELETE, and a correlated `NOT EXISTS`. (The *expression*-target ON
/// CONFLICT the native parser unblocks is simply absent from this pglet-only
/// baseline set — the bench measures the native parser only.)
const CASES: &[ParseCase] = &[
    ParseCase {
        label: "select_eq",
        sql: "SELECT id, name FROM cell WHERE id = '00000000-0000-0000-0000-000000000001'",
    },
    ParseCase {
        label: "select_jsonb_at",
        sql: "SELECT id FROM cell WHERE data @> '{\"k\":\"v\"}'",
    },
    ParseCase {
        label: "select_array_at",
        sql: "SELECT id FROM cell WHERE refs @> ARRAY['tok']",
    },
    ParseCase {
        label: "insert",
        sql: "INSERT INTO cell (id, name) \
              VALUES ('00000000-0000-0000-0000-000000000001', 'x')",
    },
    ParseCase {
        label: "insert_on_conflict",
        sql: "INSERT INTO role_grant (id, actor_id, role) \
              VALUES ('00000000-0000-0000-0000-000000000002', \
              '00000000-0000-0000-0000-000000000003', 'admin') \
              ON CONFLICT (id) DO NOTHING",
    },
    ParseCase {
        label: "update",
        sql: "UPDATE cell SET name = 'y' \
              WHERE id = '00000000-0000-0000-0000-000000000001'",
    },
    ParseCase {
        label: "delete",
        sql: "DELETE FROM cell WHERE deleted_at IS NOT NULL",
    },
    ParseCase {
        label: "select_not_exists",
        sql: "SELECT c.id FROM cell c \
              WHERE NOT EXISTS (SELECT 1 FROM role_grant g WHERE g.scope_id = c.id)",
    },
];

/// One case's measured front-end latencies (microseconds).
struct ParseRow {
    label: &'static str,
    parse_p50_us: f64,
    parse_p95_us: f64,
    plan_p50_us: f64,
    plan_p95_us: f64,
    total_p50_us: f64,
    total_p95_us: f64,
    /// Coefficient of variation of the combined parse+plan samples.
    total_cv: f64,
}

impl ParseRow {
    fn to_json(&self) -> String {
        format!(
            "{{ \"case\": \"{}\", \"parse_p50_us\": {:.3}, \"parse_p95_us\": {:.3}, \
\"plan_p50_us\": {:.3}, \"plan_p95_us\": {:.3}, \"total_p50_us\": {:.3}, \
\"total_p95_us\": {:.3}, \"total_cv\": {:.4} }}",
            self.label,
            self.parse_p50_us,
            self.parse_p95_us,
            self.plan_p50_us,
            self.plan_p95_us,
            self.total_p50_us,
            self.total_p95_us,
            self.total_cv,
        )
    }
}

/// Sorted p50/p95 (microseconds) of a latency sample set.
fn p50_p95(mut lat: Vec<f64>) -> (f64, f64) {
    lat.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    (percentile_sorted(&lat, 0.50), percentile_sorted(&lat, 0.95))
}

/// Measure the native parser's **fused** parse+plan latency for one case (the
/// native front-end has no separate parse/plan split, so `parse`/`total` coincide
/// and `plan` is reported as 0).
fn measure_case(db: &Database, case: &ParseCase) -> ParseRow {
    use pglet_sql::parser::parse as native_parse;

    for _ in 0..PARSE_WARMUP {
        std::hint::black_box(native_parse(case.sql, db.schema()).expect("warmup parse"));
    }

    let mut total_lat = Vec::with_capacity(PARSE_SAMPLES);
    for _ in 0..PARSE_SAMPLES {
        let start = Instant::now();
        let p = native_parse(case.sql, db.schema()).expect("parse");
        total_lat.push(start.elapsed().as_nanos() as f64 / 1000.0);
        std::hint::black_box(&p);
    }

    let total_cv = cv(&total_lat);
    let (total_p50_us, total_p95_us) = p50_p95(total_lat);
    ParseRow {
        label: case.label,
        parse_p50_us: total_p50_us,
        parse_p95_us: total_p95_us,
        plan_p50_us: 0.0,
        plan_p95_us: 0.0,
        total_p50_us,
        total_p95_us,
        total_cv,
    }
}

/// The `parse` scenario: front-end (parse / plan / parse+plan) latency baseline.
pub fn run_parse(update: bool) {
    use core::fmt::Write as _;

    // In-memory engine — the front end never touches storage, so no temp file or
    // persistence is needed. Build the catalog once.
    let mut db = Database::with_btree();
    for ddl in SCHEMA_SQL {
        db.execute(ddl).expect("schema DDL");
    }

    eprintln!(
        "pglet_bench: parse/plan front-end latency (native, pglet-only). \
warmup={PARSE_WARMUP} samples={PARSE_SAMPLES}; cases={}",
        CASES.len()
    );

    let mut results: Vec<ParseRow> = Vec::with_capacity(CASES.len());
    for case in CASES {
        eprintln!("  {} ...", case.label);
        results.push(measure_case(&db, case));
    }

    println!("\n=== parse/plan front-end latency (native, pglet-only) ===");
    println!(
        "warmup={PARSE_WARMUP} samples={PARSE_SAMPLES}; the perf baseline for the custom-parser swap gate"
    );
    println!(
        "{:<20} {:>10} {:>10} {:>10} {:>10} {:>10} {:>10} {:>8}",
        "case", "parse_p50", "parse_p95", "plan_p50", "plan_p95", "total_p50", "total_p95", "cv"
    );
    println!("{}", "-".repeat(96));
    for r in &results {
        println!(
            "{:<20} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>8.3}",
            r.label,
            r.parse_p50_us,
            r.parse_p95_us,
            r.plan_p50_us,
            r.plan_p95_us,
            r.total_p50_us,
            r.total_p95_us,
            r.total_cv,
        );
    }
    println!("{}", "-".repeat(96));
    println!("(all latencies in microseconds; lower is better)");

    let mut json = String::new();
    json.push_str("{\n  \"meta\": {\n");
    json.push_str("    \"benchmark\": \"parse_plan_latency\",\n");
    json.push_str("    \"lane\": \"native_inprocess\",\n");
    json.push_str("    \"layer\": \"L1\",\n");
    json.push_str("    \"engine\": \"pglet (native front end, fused parse+plan)\",\n");
    let _ = writeln!(
        json,
        "    \"warmup\": {PARSE_WARMUP},\n    \"samples\": {PARSE_SAMPLES},"
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

    write_results(&bench_results_path("parse_results.json"), &json, update);

    println!("\n=== JSON ===");
    println!("{json}");
}
