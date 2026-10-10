//! Load one or more `.sql` schema files into an in-memory pglet `Database`,
//! statement by statement, reporting OK/ERR per statement.
//!
//! The schema-load instrument: point it at the fuz spine's
//! `full_*_schema.sql` files and the ERR lines are the prioritized, index-free
//! "Complete" backlog (the SQL the spine needs that pglet can't yet load).
//! pglet-only — no real Postgres needed; we want pglet's own parse/plan errors.
//!
//!   cargo run -p pglet --example schema_load -- a.sql [b.sql ...]
//!
//! Files load in order into one `Database`, so pass any prerequisite DDL first
//! (e.g. a minimal `actor` table the cell schema's FKs reference).

use pglet::Database;
use std::process::ExitCode;

/// Strip `--` line comments and split a SQL script into individual statements
/// on `;`. Good enough for the spine schemas (no `$$` bodies, no `;`/`--`
/// inside string literals); not a general-purpose SQL splitter.
fn split_statements(sql: &str) -> Vec<String> {
    let stripped: String = sql
        .lines()
        .map(|line| line.find("--").map_or(line, |i| &line[..i]))
        .collect::<Vec<_>>()
        .join("\n");
    stripped
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Collapse a statement to a single-line label for the report.
fn label(stmt: &str) -> String {
    let one_line: String = stmt.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.len() > 72 {
        format!("{}…", &one_line[..72])
    } else {
        one_line
    }
}

fn main() -> ExitCode {
    let files: Vec<String> = std::env::args().skip(1).collect();
    if files.is_empty() {
        eprintln!("usage: schema_load <file.sql> [more.sql ...]");
        return ExitCode::FAILURE;
    }

    let mut db = Database::in_memory();
    let mut ok = 0usize;
    let mut failed = 0usize;

    for file in &files {
        let sql = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("read {file}: {e}");
                return ExitCode::FAILURE;
            }
        };
        println!("\n# {file}");
        for stmt in split_statements(&sql) {
            match db.execute(&stmt) {
                Ok(results) => {
                    ok += 1;
                    println!("  \x1b[32mOK \x1b[0m {}", label(&stmt));
                    // Print any returned rows so silent coercions are visible
                    // (e.g. a `DEFAULT gen_random_uuid()` id that comes back NULL,
                    // or a `TEXT[]` value round-tripped as plain text).
                    for r in &results {
                        if r.rows.is_empty() {
                            continue;
                        }
                        let cols: Vec<&str> = r.columns().iter().map(|c| c.name.as_str()).collect();
                        println!("       cols: {}", cols.join(", "));
                        for row in &r.rows {
                            let cells: Vec<String> = row.iter().map(|v| format!("{v:?}")).collect();
                            println!("       row:  {}", cells.join(" | "));
                        }
                    }
                }
                Err(e) => {
                    failed += 1;
                    println!("  \x1b[31mERR\x1b[0m {}", label(&stmt));
                    println!("       \x1b[31m↳ {e}\x1b[0m");
                }
            }
        }
    }

    println!("\n=== {ok} ok, {failed} failed ===");
    ExitCode::SUCCESS
}
