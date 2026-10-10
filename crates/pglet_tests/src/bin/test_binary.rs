//! Comprehensive binary format tests for tokio-postgres.
//!
//! Tests all 13 supported types in binary format, NULL handling, and parameterized queries.
//!
//! Run with: cargo run -p `pglet_tests` --bin `test_binary`
//!
//! This binary is self-contained: it builds `pglet_server`, spawns it on an open port,
//! runs all tests, then kills the server. No external server management needed.

// Test-harness binary: `.expect()`/`.unwrap()` surface failures loudly (the right behavior
// for a test runner), and float literals like `3.14` are deliberate round-trip test data,
// not the math constants `approx_constant` mistakes them for.
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::approx_constant)]

use pglet_testing::server::build_server;
use pglet_tests::{DYNAMIC_PORT_END, PGLET_TEST_PORT};
use std::net::TcpListener;
use std::process::{Child, Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;
use time::macros::datetime;
use time::{OffsetDateTime, PrimitiveDateTime};
use tokio_postgres::NoTls;

/// Global port for this test run (set once in main)
static TEST_PORT: OnceLock<u16> = OnceLock::new();

/// Find an open port in the dynamic range
// test harness: no free port in the range is an unrecoverable setup failure
#[allow(clippy::panic)]
fn find_open_port() -> u16 {
    for port in PGLET_TEST_PORT..DYNAMIC_PORT_END {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return port;
        }
    }
    panic!("No open port found in range {PGLET_TEST_PORT}-{DYNAMIC_PORT_END}");
}

/// Spawn `pglet_server` on the given port
fn spawn_server(port: u16) -> Child {
    Command::new("target/debug/pglet_server")
        .args(["--port", &port.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("Failed to spawn pglet_server")
}

/// Wait for server to accept connections
async fn wait_for_ready(port: u16) -> bool {
    let conn_str =
        format!("host=127.0.0.1 port={port} user=postgres dbname=postgres connect_timeout=1");
    for _ in 0..50 {
        // 5 seconds max
        if let Ok((client, conn)) = tokio_postgres::connect(&conn_str, NoTls).await {
            tokio::spawn(conn);
            drop(client);
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    false
}

async fn connect() -> tokio_postgres::Client {
    let port = *TEST_PORT.get().expect("TEST_PORT not set");
    let conn_str = format!("host=127.0.0.1 port={port} user=postgres dbname=postgres");
    let (client, connection) = tokio_postgres::connect(&conn_str, NoTls)
        .await
        .expect("Failed to connect to pglet");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("Connection error: {e}");
        }
    });

    client
}

// ============================================================================
// Integer Types (INT2, INT4, INT8)
// ============================================================================

async fn test_int2() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Test positive, negative, and boundary values
    let row = client
        .query_one("SELECT 32767::int2 as max_val", &[])
        .await?;
    let val: i16 = row.get(0);
    assert_eq!(val, 32767);

    let row = client
        .query_one("SELECT (-32768)::int2 as min_val", &[])
        .await?;
    let val: i16 = row.get(0);
    assert_eq!(val, -32768);

    let row = client.query_one("SELECT 0::int2 as zero", &[]).await?;
    let val: i16 = row.get(0);
    assert_eq!(val, 0);

    println!("✓ INT2 binary format");
    Ok(())
}

async fn test_int4() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one("SELECT 2147483647::int4 as max_val", &[])
        .await?;
    let val: i32 = row.get(0);
    assert_eq!(val, 2_147_483_647);

    let row = client
        .query_one("SELECT (-2147483648)::int4 as min_val", &[])
        .await?;
    let val: i32 = row.get(0);
    assert_eq!(val, -2_147_483_648);

    let row = client.query_one("SELECT 42::int4 as meaning", &[]).await?;
    let val: i32 = row.get(0);
    assert_eq!(val, 42);

    println!("✓ INT4 binary format");
    Ok(())
}

async fn test_int8() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one("SELECT 9223372036854775807::int8 as max_val", &[])
        .await?;
    let val: i64 = row.get(0);
    assert_eq!(val, 9_223_372_036_854_775_807_i64);

    let row = client
        .query_one("SELECT (-9223372036854775808)::int8 as min_val", &[])
        .await?;
    let val: i64 = row.get(0);
    assert_eq!(val, -9_223_372_036_854_775_808_i64);

    println!("✓ INT8 binary format");
    Ok(())
}

// ============================================================================
// Floating Point Types (FLOAT4, FLOAT8)
// ============================================================================

async fn test_float4() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one("SELECT 3.14::float4 as pi_approx", &[])
        .await?;
    let val: f32 = row.get(0);
    assert!((val - 3.14).abs() < 0.001);

    let row = client
        .query_one("SELECT (-123.456)::float4 as neg", &[])
        .await?;
    let val: f32 = row.get(0);
    assert!((val - (-123.456)).abs() < 0.01);

    let row = client.query_one("SELECT 0.0::float4 as zero", &[]).await?;
    let val: f32 = row.get(0);
    assert!((val - 0.0).abs() < f32::EPSILON);

    println!("✓ FLOAT4 binary format");
    Ok(())
}

async fn test_float8() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one("SELECT 3.141592653589793::float8 as pi", &[])
        .await?;
    let val: f64 = row.get(0);
    assert!((val - std::f64::consts::PI).abs() < 1e-15);

    let row = client
        .query_one("SELECT (-1e100)::float8 as big_neg", &[])
        .await?;
    let val: f64 = row.get(0);
    assert!((val - (-1e100)).abs() < 1e90);

    println!("✓ FLOAT8 binary format");
    Ok(())
}

// ============================================================================
// Boolean
// ============================================================================

async fn test_bool() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client.query_one("SELECT true::bool as t", &[]).await?;
    let val: bool = row.get(0);
    assert!(val);

    let row = client.query_one("SELECT false::bool as f", &[]).await?;
    let val: bool = row.get(0);
    assert!(!val);

    println!("✓ BOOL binary format");
    Ok(())
}

// ============================================================================
// Text Types (TEXT, VARCHAR)
// ============================================================================

async fn test_text() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one("SELECT 'hello world'::text as greeting", &[])
        .await?;
    let val: String = row.get(0);
    assert_eq!(val, "hello world");

    // Empty string
    let row = client.query_one("SELECT ''::text as empty", &[]).await?;
    let val: String = row.get(0);
    assert_eq!(val, "");

    // Unicode
    let row = client
        .query_one("SELECT '你好世界 🌍'::text as unicode", &[])
        .await?;
    let val: String = row.get(0);
    assert_eq!(val, "你好世界 🌍");

    println!("✓ TEXT binary format");
    Ok(())
}

// ============================================================================
// BYTEA
// ============================================================================

async fn test_bytea() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Test hex format input
    let row = client
        .query_one(r"SELECT '\x0001ff'::bytea as data", &[])
        .await?;
    let val: Vec<u8> = row.get(0);
    assert_eq!(val, vec![0, 1, 255]);

    // Empty bytea
    let row = client
        .query_one(r"SELECT '\x'::bytea as empty", &[])
        .await?;
    let val: Vec<u8> = row.get(0);
    assert!(val.is_empty());

    // All byte values test
    let row = client
        .query_one(r"SELECT '\xdeadbeef'::bytea as dead", &[])
        .await?;
    let val: Vec<u8> = row.get(0);
    assert_eq!(val, vec![0xde, 0xad, 0xbe, 0xef]);

    println!("✓ BYTEA binary format");
    Ok(())
}

// ============================================================================
// Timestamp Types
// ============================================================================

async fn test_timestamp() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Test a known timestamp
    let row = client
        .query_one("SELECT '2024-06-15 12:30:45'::timestamp as ts", &[])
        .await?;
    let val: PrimitiveDateTime = row.get(0);
    assert_eq!(val.to_string(), "2024-06-15 12:30:45.0");

    // Epoch (2000-01-01 is Postgres epoch)
    let row = client
        .query_one("SELECT '2000-01-01 00:00:00'::timestamp as epoch", &[])
        .await?;
    let val: PrimitiveDateTime = row.get(0);
    assert_eq!(val.to_string(), "2000-01-01 0:00:00.0");

    println!("✓ TIMESTAMP binary format");
    Ok(())
}

async fn test_timestamptz() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Test timestamp with timezone
    let row = client
        .query_one("SELECT '2024-06-15 12:30:45+00'::timestamptz as ts", &[])
        .await?;
    let val: OffsetDateTime = row.get(0);
    assert_eq!(val, datetime!(2024-06-15 12:30:45 UTC));

    println!("✓ TIMESTAMPTZ binary format");
    Ok(())
}

// ============================================================================
// JSONB
// ============================================================================

async fn test_jsonb() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Simple object
    let row = client
        .query_one(
            r#"SELECT '{"name": "Alice", "age": 30}'::jsonb as data"#,
            &[],
        )
        .await?;
    let val: serde_json::Value = row.get(0);
    assert_eq!(val["name"], "Alice");
    assert_eq!(val["age"], 30);

    // Array
    let row = client
        .query_one(r"SELECT '[1, 2, 3]'::jsonb as arr", &[])
        .await?;
    let val: serde_json::Value = row.get(0);
    assert!(val.is_array());
    assert_eq!(val.as_array().unwrap().len(), 3);

    // Nested structure
    let row = client
        .query_one(
            r#"SELECT '{"outer": {"inner": true}}'::jsonb as nested"#,
            &[],
        )
        .await?;
    let val: serde_json::Value = row.get(0);
    assert_eq!(val["outer"]["inner"], true);

    // Null JSON value
    let row = client
        .query_one(r"SELECT 'null'::jsonb as null_json", &[])
        .await?;
    let val: serde_json::Value = row.get(0);
    assert!(val.is_null());

    println!("✓ JSONB binary format");
    Ok(())
}

// ============================================================================
// UUID
// ============================================================================

async fn test_uuid() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Standard UUID format
    let row = client
        .query_one(
            "SELECT '550e8400-e29b-41d4-a716-446655440000'::uuid as id",
            &[],
        )
        .await?;
    let val: uuid::Uuid = row.get(0);
    assert_eq!(val.to_string(), "550e8400-e29b-41d4-a716-446655440000");

    // Nil UUID
    let row = client
        .query_one(
            "SELECT '00000000-0000-0000-0000-000000000000'::uuid as nil",
            &[],
        )
        .await?;
    let val: uuid::Uuid = row.get(0);
    assert!(val.is_nil());

    // Max UUID
    let row = client
        .query_one(
            "SELECT 'ffffffff-ffff-ffff-ffff-ffffffffffff'::uuid as max",
            &[],
        )
        .await?;
    let val: uuid::Uuid = row.get(0);
    assert_eq!(val.to_string(), "ffffffff-ffff-ffff-ffff-ffffffffffff");

    println!("✓ UUID binary format");
    Ok(())
}

// ============================================================================
// NULL Value Tests
// ============================================================================

async fn test_null_values() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Test NULL for each type
    let row = client
        .query_one(
            "SELECT NULL::int2, NULL::int4, NULL::int8, NULL::float4, NULL::float8",
            &[],
        )
        .await?;
    assert!(row.get::<_, Option<i16>>(0).is_none());
    assert!(row.get::<_, Option<i32>>(1).is_none());
    assert!(row.get::<_, Option<i64>>(2).is_none());
    assert!(row.get::<_, Option<f32>>(3).is_none());
    assert!(row.get::<_, Option<f64>>(4).is_none());

    let row = client
        .query_one("SELECT NULL::bool, NULL::text, NULL::bytea", &[])
        .await?;
    assert!(row.get::<_, Option<bool>>(0).is_none());
    assert!(row.get::<_, Option<String>>(1).is_none());
    assert!(row.get::<_, Option<Vec<u8>>>(2).is_none());

    let row = client
        .query_one("SELECT NULL::timestamp, NULL::timestamptz", &[])
        .await?;
    assert!(row.get::<_, Option<PrimitiveDateTime>>(0).is_none());
    assert!(row.get::<_, Option<OffsetDateTime>>(1).is_none());

    let row = client
        .query_one("SELECT NULL::jsonb, NULL::uuid", &[])
        .await?;
    assert!(row.get::<_, Option<serde_json::Value>>(0).is_none());
    assert!(row.get::<_, Option<uuid::Uuid>>(1).is_none());

    println!("✓ NULL values binary format");
    Ok(())
}

// ============================================================================
// Parameterized Query Tests
//
// tokio-postgres sends Parse with empty param_types=[] and relies on the server
// to infer parameter types (reported via Describe/ParameterDescription, then used
// to decode the bound values). pglet infers from explicit `$N::type` casts and
// from usage context (column comparisons, ANY/@>, IN, LIMIT). The spine-shaped
// cases below exercise the context-inference path the fuz spine depends on.
// ============================================================================

async fn test_params_int() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // INT4 parameters
    let row = client
        .query_one("SELECT $1::int4 + $2::int4 as sum", &[&42i32, &8i32])
        .await?;
    assert_eq!(row.get::<_, i32>(0), 50);

    // INT8 parameters
    let row = client
        .query_one(
            "SELECT $1::int8 * $2::int8 as product",
            &[&1_000_000_i64, &1_000_000_i64],
        )
        .await?;
    assert_eq!(row.get::<_, i64>(0), 1_000_000_000_000_i64);

    println!("✓ Parameterized INT queries");
    Ok(())
}

async fn test_params_text() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Text parameter
    let row = client
        .query_one("SELECT $1::text as echo", &[&"hello from params"])
        .await?;
    assert_eq!(row.get::<_, String>(0), "hello from params");

    // Unicode in parameter
    let row = client
        .query_one("SELECT $1::text as unicode", &[&"日本語 🎉"])
        .await?;
    assert_eq!(row.get::<_, String>(0), "日本語 🎉");

    println!("✓ Parameterized TEXT queries");
    Ok(())
}

async fn test_params_bool() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one("SELECT $1::bool AND $2::bool as result", &[&true, &false])
        .await?;
    assert!(!row.get::<_, bool>(0));

    let row = client
        .query_one("SELECT $1::bool OR $2::bool as result", &[&true, &false])
        .await?;
    assert!(row.get::<_, bool>(0));

    println!("✓ Parameterized BOOL queries");
    Ok(())
}

async fn test_params_null() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // NULL parameter
    let row = client
        .query_one("SELECT $1::int4 as maybe_null", &[&None::<i32>])
        .await?;
    assert!(row.get::<_, Option<i32>>(0).is_none());

    // Mix of NULL and non-NULL
    let row = client
        .query_one(
            "SELECT $1::int4, $2::text, $3::bool",
            &[&Some(42i32), &None::<String>, &Some(true)],
        )
        .await?;
    assert_eq!(row.get::<_, Option<i32>>(0), Some(42));
    assert!(row.get::<_, Option<String>>(1).is_none());
    assert_eq!(row.get::<_, Option<bool>>(2), Some(true));

    println!("✓ Parameterized NULL queries");
    Ok(())
}

async fn test_params_float() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one(
            "SELECT $1::float8 + $2::float8 as sum",
            &[&3.14f64, &2.86f64],
        )
        .await?;
    let val: f64 = row.get(0);
    assert!((val - 6.0).abs() < 1e-10);

    println!("✓ Parameterized FLOAT queries");
    Ok(())
}

async fn test_params_bytea() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let input = vec![0xde, 0xad, 0xbe, 0xef];
    let row = client
        .query_one("SELECT $1::bytea as data", &[&input])
        .await?;
    let val: Vec<u8> = row.get(0);
    assert_eq!(val, input);

    println!("✓ Parameterized BYTEA queries");
    Ok(())
}

async fn test_params_uuid() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let id = uuid::Uuid::parse_str("550e8400-e29b-41d4-a716-446655440000").unwrap();
    let row = client.query_one("SELECT $1::uuid as id", &[&id]).await?;
    let val: uuid::Uuid = row.get(0);
    assert_eq!(val, id);

    println!("✓ Parameterized UUID queries");
    Ok(())
}

async fn test_params_jsonb() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let json_val = serde_json::json!({"key": "value", "num": 42});
    let row = client
        .query_one("SELECT $1::jsonb as data", &[&json_val])
        .await?;
    let val: serde_json::Value = row.get(0);
    assert_eq!(val["key"], "value");
    assert_eq!(val["num"], 42);

    println!("✓ Parameterized JSONB queries");
    Ok(())
}

async fn test_params_timestamp() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let ts = datetime!(2024-06-15 12:30:45);
    let row = client
        .query_one("SELECT $1::timestamp as ts", &[&ts])
        .await?;
    let val: PrimitiveDateTime = row.get(0);
    assert_eq!(val, ts);

    println!("✓ Parameterized TIMESTAMP queries");
    Ok(())
}

// ============================================================================
// Spine-shaped Parameter Tests
//
// These mirror the patterns the fuz spine's hand-written SQL issues over the
// cell/fact schema. They are the param-binding conformance target: array params,
// `ANY`/`@>`/`unnest`, untyped column comparisons (context inference), `IN`,
// double casts, enum casts, and tri-state optional filters.
// ============================================================================

/// `id = ANY($1::uuid[])` (cast) and `id = ANY($1)` (inferred uuid[]).
/// Mirrors `query_cell_load_many` / grant `load_many`.
async fn test_params_any_uuid_array() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;
    let a = uuid::Uuid::parse_str("00000000-0000-0000-0000-0000000000a1").unwrap();
    let b = uuid::Uuid::parse_str("00000000-0000-0000-0000-0000000000b2").unwrap();
    let c = uuid::Uuid::parse_str("00000000-0000-0000-0000-0000000000c3").unwrap();
    client
        .batch_execute(
            "CREATE TABLE pt_any (id uuid);
             INSERT INTO pt_any (id) VALUES \
             ('00000000-0000-0000-0000-0000000000a1'),\
             ('00000000-0000-0000-0000-0000000000b2'),\
             ('00000000-0000-0000-0000-0000000000c3');",
        )
        .await?;

    // Explicit cast form.
    let rows = client
        .query(
            "SELECT id FROM pt_any WHERE id = ANY($1::uuid[]) ORDER BY id",
            &[&vec![a, c]],
        )
        .await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, uuid::Uuid>(0), a);
    assert_eq!(rows[1].get::<_, uuid::Uuid>(0), c);

    // Bare form — `$1` is inferred as uuid[] from the column.
    let rows = client
        .query("SELECT id FROM pt_any WHERE id = ANY($1)", &[&vec![b]])
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, uuid::Uuid>(0), b);

    println!("✓ Param ANY(uuid[]) (cast + inferred)");
    Ok(())
}

/// `refs @> ARRAY[$1]::text[]` and `refs @> $1` (inferred text[]).
/// Mirrors `cell_list`'s refs-containment and the fact orphan predicate.
async fn test_params_containment() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;
    client
        .batch_execute(
            "CREATE TABLE pt_refs (id int4, refs text[]);
             INSERT INTO pt_refs (id, refs) VALUES (1, '{a,b}'), (2, '{c}');",
        )
        .await?;

    // Bound single-element array containment.
    let rows = client
        .query(
            "SELECT id FROM pt_refs WHERE refs @> $1 ORDER BY id",
            &[&vec!["a"]],
        )
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, i32>(0), 1);

    // `ARRAY[$1]::text[]` — the element parameter is typed text by the cast.
    let rows = client
        .query(
            "SELECT id FROM pt_refs WHERE refs @> ARRAY[$1]::text[]",
            &[&"c"],
        )
        .await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].get::<_, i32>(0), 2);

    println!("✓ Param @> containment (bound array + ARRAY[$n])");
    Ok(())
}

/// `unnest($1::text[], $2::uuid[])` zipped into rows. Mirrors the caller
/// role-grants CTE in `cell_list`.
async fn test_params_unnest() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;
    let id1 = uuid::Uuid::parse_str("00000000-0000-0000-0000-0000000000d4").unwrap();
    let id2 = uuid::Uuid::parse_str("00000000-0000-0000-0000-0000000000e5").unwrap();

    let rows = client
        .query(
            "SELECT role, id FROM unnest($1::text[], $2::uuid[]) AS p(role, id) ORDER BY role",
            &[&vec!["admin", "member"], &vec![id1, id2]],
        )
        .await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].get::<_, String>(0), "admin");
    assert_eq!(rows[0].get::<_, uuid::Uuid>(1), id1);
    assert_eq!(rows[1].get::<_, String>(0), "member");
    assert_eq!(rows[1].get::<_, uuid::Uuid>(1), id2);

    println!("✓ Param unnest(text[], uuid[])");
    Ok(())
}

/// Untyped `WHERE id = $1` (uuid) — the column-context inference case. The
/// parameter has no cast; its type must come from the `id` column. Mirrors
/// `query_cell_get`.
async fn test_params_untyped_column_compare() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;
    let id = uuid::Uuid::parse_str("00000000-0000-0000-0000-00000000f001").unwrap();
    client
        .batch_execute(
            "CREATE TABLE pt_get (id uuid, name text);
             INSERT INTO pt_get (id, name) VALUES \
             ('00000000-0000-0000-0000-00000000f001', 'alice');",
        )
        .await?;

    let row = client
        .query_one("SELECT name FROM pt_get WHERE id = $1", &[&id])
        .await?;
    assert_eq!(row.get::<_, String>(0), "alice");

    println!("✓ Param untyped WHERE id = $1 (column inference)");
    Ok(())
}

/// `$1::text::jsonb` — a double cast (the spine sends JSON as a text param).
/// Mirrors `query_cell_create`'s `data` binding.
async fn test_params_text_to_jsonb() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one(
            "SELECT ($1::text::jsonb ->> 'kind') as kind",
            &[&r#"{"kind":"doc"}"#],
        )
        .await?;
    assert_eq!(row.get::<_, String>(0), "doc");

    println!("✓ Param $1::text::jsonb (double cast)");
    Ok(())
}

/// `$1::text::timestamptz` — RFC3339 string param cast to a timestamp. Mirrors
/// the fact orphan `older_than` filter.
async fn test_params_text_to_timestamptz() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    let row = client
        .query_one(
            "SELECT ($1::text::timestamptz)::text as t",
            &[&"2024-06-15T12:30:45Z"],
        )
        .await?;
    // A `timestamptz` cast to `text` keeps its `+00`, as 18.6's does.
    assert_eq!(row.get::<_, String>(0), "2024-06-15 12:30:45+00");

    println!("✓ Param $1::text::timestamptz");
    Ok(())
}

/// `$1::text::<enum>` — a string param cast to a user ENUM. Mirrors the
/// `visibility` binding in `query_cell_create`.
async fn test_params_text_to_enum() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;
    client
        .batch_execute("CREATE TYPE pt_vis AS ENUM ('private', 'public');")
        .await?;

    let row = client
        .query_one("SELECT ($1::text::pt_vis)::text as v", &[&"public"])
        .await?;
    assert_eq!(row.get::<_, String>(0), "public");

    println!("✓ Param $1::text::<enum>");
    Ok(())
}

/// Tri-state optional filter `($1::text IS NULL OR kind = $1::text)` — NULL turns
/// the filter off. Mirrors the pervasive `cell_list` optional predicates.
async fn test_params_optional_filter() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;
    client
        .batch_execute(
            "CREATE TABLE pt_opt (id int4, kind text);
             INSERT INTO pt_opt (id, kind) VALUES (1, 'a'), (2, 'b');",
        )
        .await?;

    // NULL parameter → filter disabled, all rows.
    let none: Option<&str> = None;
    let row = client
        .query_one(
            "SELECT count(*)::int8 FROM pt_opt WHERE ($1::text IS NULL OR kind = $1::text)",
            &[&none],
        )
        .await?;
    assert_eq!(row.get::<_, i64>(0), 2);

    // Present parameter → filter active.
    let row = client
        .query_one(
            "SELECT count(*)::int8 FROM pt_opt WHERE ($1::text IS NULL OR kind = $1::text)",
            &[&Some("a")],
        )
        .await?;
    assert_eq!(row.get::<_, i64>(0), 1);

    println!("✓ Param tri-state optional filter ($1 IS NULL OR ...)");
    Ok(())
}

// ============================================================================
// Multi-value Tests
// ============================================================================

async fn test_multiple_columns() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Test multiple types in one query
    let row = client
        .query_one(
            "SELECT 42::int4, 'hello'::text, true::bool, 3.14::float8",
            &[],
        )
        .await?;

    assert_eq!(row.get::<_, i32>(0), 42);
    assert_eq!(row.get::<_, String>(1), "hello");
    assert!(row.get::<_, bool>(2));
    assert!((row.get::<_, f64>(3) - 3.14).abs() < 0.001);

    println!("✓ Multiple column types in single row");
    Ok(())
}

// ============================================================================
// Main runner
// ============================================================================

#[tokio::main]
async fn main() {
    // Build server first (fast if already up-to-date)
    if !build_server() {
        eprintln!("[test] Build failed!");
        std::process::exit(1);
    }

    // Find open port and spawn server
    let port = find_open_port();
    TEST_PORT.set(port).expect("TEST_PORT already set");
    eprintln!("[test] Spawning pglet_server on port {port}...");
    let mut server = spawn_server(port);

    // Wait for server to be ready
    if !wait_for_ready(port).await {
        eprintln!("[test] Server failed to start!");
        server.kill().ok();
        std::process::exit(1);
    }
    eprintln!("[test] Server ready\n");

    println!("=== Binary Format Test Suite ===\n");

    let mut passed = 0;
    let mut failed = 0;

    macro_rules! run_test {
        ($name:expr, $test:expr) => {
            match $test.await {
                Ok(()) => passed += 1,
                Err(e) => {
                    eprintln!("✗ {}: {}", $name, e);
                    failed += 1;
                }
            }
        };
    }

    println!("--- Integer Types ---");
    run_test!("INT2", test_int2());
    run_test!("INT4", test_int4());
    run_test!("INT8", test_int8());

    println!("\n--- Floating Point Types ---");
    run_test!("FLOAT4", test_float4());
    run_test!("FLOAT8", test_float8());

    println!("\n--- Boolean ---");
    run_test!("BOOL", test_bool());

    println!("\n--- Text Types ---");
    run_test!("TEXT", test_text());

    println!("\n--- Binary Data ---");
    run_test!("BYTEA", test_bytea());

    println!("\n--- Timestamps ---");
    run_test!("TIMESTAMP", test_timestamp());
    run_test!("TIMESTAMPTZ", test_timestamptz());

    println!("\n--- JSON ---");
    run_test!("JSONB", test_jsonb());

    println!("\n--- UUID ---");
    run_test!("UUID", test_uuid());

    println!("\n--- NULL Values ---");
    run_test!("NULL values", test_null_values());

    println!("\n--- Parameterized Queries ---");
    run_test!("Params INT", test_params_int());
    run_test!("Params TEXT", test_params_text());
    run_test!("Params BOOL", test_params_bool());
    run_test!("Params NULL", test_params_null());
    run_test!("Params FLOAT", test_params_float());
    run_test!("Params BYTEA", test_params_bytea());
    run_test!("Params UUID", test_params_uuid());
    run_test!("Params JSONB", test_params_jsonb());
    run_test!("Params TIMESTAMP", test_params_timestamp());

    println!("\n--- Spine-shaped Parameters ---");
    run_test!("Params ANY(uuid[])", test_params_any_uuid_array());
    run_test!("Params @> containment", test_params_containment());
    run_test!("Params unnest", test_params_unnest());
    run_test!(
        "Params untyped column",
        test_params_untyped_column_compare()
    );
    run_test!("Params text::jsonb", test_params_text_to_jsonb());
    run_test!(
        "Params text::timestamptz",
        test_params_text_to_timestamptz()
    );
    run_test!("Params text::enum", test_params_text_to_enum());
    run_test!("Params optional filter", test_params_optional_filter());

    println!("\n--- Multi-value ---");
    run_test!("Multiple columns", test_multiple_columns());

    println!("\n=== Results ===");
    println!("Passed: {passed}");
    println!("Failed: {failed}");

    // Clean up server — kill and reap so it can't linger as a zombie
    eprintln!("\n[test] Shutting down server...");
    server.kill().ok();
    server.wait().ok();

    if failed > 0 {
        std::process::exit(1);
    }
}
