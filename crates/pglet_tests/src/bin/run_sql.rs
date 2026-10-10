//! Simple SQL execution tool for pglet.
//!
//! Starts a pglet server, runs SQL statements, outputs JSON results.
//!
//! Usage:
//!   cargo run -p `pglet_tests` --bin `run_sql` -- "SELECT 1 + 1"
//!   cargo run -p `pglet_tests` --bin `run_sql` -- "CREATE TABLE t (id INT); INSERT INTO t VALUES (1); SELECT * FROM t"
//!
//! Output format:
//!   `{ "columns": ["?column?"], "rows": [[2]], "command_tag": "SELECT 1" }`

// Test-harness binary: `.expect()`/`.unwrap()` surface setup failures loudly, which is the
// right behavior for a one-shot test runner rather than threading `Result` everywhere.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use pglet_testing::server::build_server;
use serde::Serialize;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};
use tokio_postgres::NoTls;

#[derive(Serialize)]
struct QueryResult {
    columns: Vec<String>,
    rows: Vec<HashMap<String, serde_json::Value>>,
    command_tag: String,
}

#[derive(Serialize)]
struct ErrorResult {
    error: String,
    code: Option<String>,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Output {
    Success(QueryResult),
    Error(ErrorResult),
}

fn find_open_port() -> u16 {
    for port in 5440..5500 {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_err() {
            return port;
        }
    }
    5440
}

fn spawn_server(port: u16) -> Child {
    Command::new("target/debug/pglet_server")
        .args(["--port", &port.to_string()])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("Failed to spawn pglet_server")
}

async fn wait_for_ready(port: u16) -> bool {
    let start = Instant::now();
    let timeout = Duration::from_secs(10);

    while start.elapsed() < timeout {
        if TcpStream::connect(format!("127.0.0.1:{port}")).is_ok() {
            // Give server a moment to fully initialize
            tokio::time::sleep(Duration::from_millis(100)).await;
            return true;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    false
}

fn parse_value(s: &str) -> serde_json::Value {
    // Try integer
    if let Ok(n) = s.parse::<i64>() {
        return serde_json::Value::Number(n.into());
    }
    // Try float
    if let Ok(f) = s.parse::<f64>()
        && let Some(n) = serde_json::Number::from_f64(f)
    {
        return serde_json::Value::Number(n);
    }
    // Try bool
    match s.to_lowercase().as_str() {
        "t" | "true" => return serde_json::Value::Bool(true),
        "f" | "false" => return serde_json::Value::Bool(false),
        _ => {}
    }
    // Default to string
    serde_json::Value::String(s.to_string())
}

async fn run_sql(port: u16, sql: &str) -> Vec<Output> {
    let conn_str = format!("host=127.0.0.1 port={port} user=postgres dbname=postgres");

    let (client, conn) = match tokio_postgres::connect(&conn_str, NoTls).await {
        Ok(c) => c,
        Err(e) => {
            return vec![Output::Error(ErrorResult {
                error: e.to_string(),
                code: None,
            })];
        }
    };

    tokio::spawn(conn);

    let mut results = Vec::new();

    // Split by semicolons but handle quotes
    for stmt in split_statements(sql) {
        let stmt = stmt.trim();
        if stmt.is_empty() {
            continue;
        }

        match client.simple_query(stmt).await {
            Ok(msgs) => {
                let mut columns = Vec::new();
                let mut rows = Vec::new();
                let mut command_tag = String::new();

                for msg in msgs {
                    match msg {
                        tokio_postgres::SimpleQueryMessage::Row(row) => {
                            if columns.is_empty() {
                                columns =
                                    row.columns().iter().map(|c| c.name().to_string()).collect();
                            }
                            let mut map = HashMap::new();
                            for (i, col) in row.columns().iter().enumerate() {
                                let val = row.get(i).map_or(serde_json::Value::Null, parse_value);
                                map.insert(col.name().to_string(), val);
                            }
                            rows.push(map);
                        }
                        tokio_postgres::SimpleQueryMessage::CommandComplete(tag) => {
                            command_tag =
                                format!("{} {tag}", stmt.split_whitespace().next().unwrap_or(""));
                            if command_tag.starts_with("SELECT") {
                                command_tag = format!("SELECT {}", rows.len());
                            }
                        }
                        _ => {}
                    }
                }

                results.push(Output::Success(QueryResult {
                    columns,
                    rows,
                    command_tag,
                }));
            }
            Err(e) => {
                results.push(Output::Error(ErrorResult {
                    error: e.to_string(),
                    code: e.code().map(|c| c.code().to_string()),
                }));
            }
        }
    }

    results
}

fn split_statements(sql: &str) -> Vec<&str> {
    // Simple split by semicolon - doesn't handle all edge cases
    sql.split(';').collect()
}

#[tokio::main]
async fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        eprintln!("Usage: run_sql \"SQL statement(s)\"");
        eprintln!("Example: run_sql \"SELECT 1 + 1\"");
        eprintln!(
            "Example: run_sql \"CREATE TABLE t (id INT); INSERT INTO t VALUES (1); SELECT * FROM t\""
        );
        std::process::exit(1);
    }

    let sql = &args[1];

    // Build server if needed
    eprintln!("[run_sql] Building server...");
    if !build_server() {
        eprintln!("[run_sql] Build failed!");
        std::process::exit(1);
    }

    // Find port and start server
    let port = find_open_port();
    eprintln!("[run_sql] Starting server on port {port}...");
    let mut server = spawn_server(port);

    // Wait for server to be ready
    if !wait_for_ready(port).await {
        eprintln!("[run_sql] Server failed to start!");
        // Print server stderr for debugging
        if let Some(stderr) = server.stderr.take() {
            let reader = BufReader::new(stderr);
            for line in reader.lines().take(10).flatten() {
                eprintln!("  {line}");
            }
        }
        server.kill().ok();
        std::process::exit(1);
    }

    eprintln!("[run_sql] Running SQL...\n");

    // Run SQL
    let results = run_sql(port, sql).await;

    // Output JSON
    for result in &results {
        println!("{}", serde_json::to_string_pretty(result).unwrap());
    }

    // Cleanup — kill and reap the child so it can't linger as a zombie
    server.kill().ok();
    server.wait().ok();
}
