//! Concurrent client tests for MVCC validation.
//!
//! Tests snapshot isolation, dirty read prevention, and connection lifecycle
//! across multiple simultaneous connections.
//!
//! It is also the only home a **cross-connection** test can have: the fixture harness
//! drives one connection and its grammar has no directive naming a second, so two
//! clients whose statements interleave is not expressible there. The writer block below
//! is that set — at most one connection's block stages writes, a second writer waits for
//! it and then answers against what it committed or took back, and a block decides its
//! writes against what is committed when it takes the writer.
//!
//! Run with: cargo run -p `pglet_tests` --bin `test_concurrent`
//!
//! Self-contained: builds server, spawns on open port, runs tests, kills server.
//!
//! Note: Parallel tests require consistent lock ordering in the server
//! (schema first, then storage) to prevent ABBA deadlock.

// Test-harness binary: `.expect()` surfaces setup failures loudly, the right behavior for a
// test runner rather than threading `Result` through every assertion.
#![allow(clippy::expect_used)]

use pglet_testing::server::{
    build_server, find_open_port, hard_kill, spawn_server, wait_for_ready,
};
use pglet_tests::{DYNAMIC_PORT_END, PGLET_TEST_PORT};
use std::sync::OnceLock;
use std::time::Duration;
use tokio_postgres::Client;

/// Global port for this test run
static TEST_PORT: OnceLock<u16> = OnceLock::new();

/// Connect to this run's server (the port is set once in `main`).
async fn connect() -> Client {
    let port = *TEST_PORT.get().expect("TEST_PORT not set");
    pglet_testing::server::connect(port).await
}

/// Helper to run SQL and ignore errors (for cleanup)
async fn exec(client: &Client, sql: &str) {
    let _ = client.simple_query(sql).await;
}

/// Helper to run SQL and get scalar result
async fn query_one_i32(client: &Client, sql: &str) -> i32 {
    let row = client.query_one(sql, &[]).await.expect("query failed");
    row.get(0)
}

/// Helper to count rows
async fn count(client: &Client, table: &str) -> i64 {
    let row = client
        .query_one(&format!("SELECT COUNT(*) FROM {table}"), &[])
        .await
        .expect("count failed");
    row.get(0)
}

/// Run `sql` and return the SQLSTATE it raised, or `None` if it succeeded.
///
/// `exec` swallows every error, so a test asserting a **refusal** cannot use it and
/// must come through here. The code is the contract and the message is not: pglet's
/// wording is free to differ from 18.6's, `23505` is not.
async fn sqlstate(client: &Client, sql: &str) -> Option<String> {
    client
        .simple_query(sql)
        .await
        .err()
        .and_then(|e| e.code().map(|c| c.code().to_string()))
}

/// Every row `sql` returns, as its single text column.
///
/// Spelled over `simple_query` so a test reads the table through the *text* protocol
/// the psql-level measurements used, and so a NULL renders as whatever the query
/// makes of it rather than needing a typed decode.
async fn text_rows(client: &Client, sql: &str) -> Vec<String> {
    client
        .simple_query(sql)
        .await
        .expect("query failed")
        .iter()
        .filter_map(|m| match m {
            tokio_postgres::SimpleQueryMessage::Row(r) => Some(r.get(0).unwrap_or("").to_string()),
            _ => None,
        })
        .collect()
}

// ============================================================================
// Test: Dirty Read Prevention
// ============================================================================

async fn test_dirty_read_prevention() -> Result<(), Box<dyn std::error::Error>> {
    let client1 = connect().await;
    let client2 = connect().await;

    // Setup
    exec(&client1, "DROP TABLE IF EXISTS dirty_test").await;
    exec(
        &client1,
        "CREATE TABLE dirty_test (id INT PRIMARY KEY, value INT)",
    )
    .await;
    exec(&client1, "INSERT INTO dirty_test VALUES (1, 100)").await;

    // Client 1 starts transaction and updates
    exec(&client1, "BEGIN").await;
    exec(&client1, "UPDATE dirty_test SET value = 200 WHERE id = 1").await;

    // Client 2 should NOT see the uncommitted change
    let value: i32 = query_one_i32(&client2, "SELECT value FROM dirty_test WHERE id = 1").await;
    assert_eq!(
        value, 100,
        "Client 2 should see original value (100), not uncommitted (200)"
    );

    // Commit and verify client 2 now sees it
    exec(&client1, "COMMIT").await;
    let value: i32 = query_one_i32(&client2, "SELECT value FROM dirty_test WHERE id = 1").await;
    assert_eq!(value, 200, "Client 2 should see committed value (200)");

    // Cleanup
    exec(&client1, "DROP TABLE dirty_test").await;

    println!("✓ Dirty read prevention");
    Ok(())
}

// ============================================================================
// Test: Snapshot Isolation
// ============================================================================

async fn test_snapshot_isolation() -> Result<(), Box<dyn std::error::Error>> {
    let client1 = connect().await;
    let client2 = connect().await;

    // Setup
    exec(&client1, "DROP TABLE IF EXISTS snapshot_test").await;
    exec(
        &client1,
        "CREATE TABLE snapshot_test (id INT PRIMARY KEY, value INT)",
    )
    .await;
    exec(&client1, "INSERT INTO snapshot_test VALUES (1, 100)").await;

    // Client 2 starts a transaction (takes snapshot)
    exec(&client2, "BEGIN").await;
    let initial: i32 =
        query_one_i32(&client2, "SELECT value FROM snapshot_test WHERE id = 1").await;
    assert_eq!(initial, 100);

    // Client 1 updates and commits
    exec(
        &client1,
        "UPDATE snapshot_test SET value = 200 WHERE id = 1",
    )
    .await;

    // Client 2 should still see the old value (snapshot isolation)
    let during: i32 = query_one_i32(&client2, "SELECT value FROM snapshot_test WHERE id = 1").await;
    assert_eq!(
        during, 100,
        "Client 2 should see snapshot value (100), not committed (200)"
    );

    // Client 2 commits, starts new transaction, now sees new value
    exec(&client2, "COMMIT").await;
    let after: i32 = query_one_i32(&client2, "SELECT value FROM snapshot_test WHERE id = 1").await;
    assert_eq!(after, 200, "After commit, client 2 should see new value");

    // Cleanup
    exec(&client1, "DROP TABLE snapshot_test").await;

    println!("✓ Snapshot isolation");
    Ok(())
}

// ============================================================================
// Test: Read Your Own Writes
// ============================================================================

async fn test_read_your_writes() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    // Setup
    exec(&client, "DROP TABLE IF EXISTS ryw_test").await;
    exec(
        &client,
        "CREATE TABLE ryw_test (id INT PRIMARY KEY, value INT)",
    )
    .await;

    // Start transaction
    exec(&client, "BEGIN").await;

    // Insert within transaction
    exec(&client, "INSERT INTO ryw_test VALUES (1, 100)").await;

    // Should see own uncommitted insert
    let value: i32 = query_one_i32(&client, "SELECT value FROM ryw_test WHERE id = 1").await;
    assert_eq!(value, 100, "Should see own uncommitted insert");

    // Update within transaction
    exec(&client, "UPDATE ryw_test SET value = 200 WHERE id = 1").await;

    // Should see own uncommitted update
    let value: i32 = query_one_i32(&client, "SELECT value FROM ryw_test WHERE id = 1").await;
    assert_eq!(value, 200, "Should see own uncommitted update");

    exec(&client, "COMMIT").await;

    // Cleanup
    exec(&client, "DROP TABLE ryw_test").await;

    println!("✓ Read your own writes");
    Ok(())
}

// ============================================================================
// Test: Rollback Visibility
// ============================================================================

/// A statement that fails inside `BEGIN … COMMIT` aborts the block, and the
/// `COMMIT` after it publishes nothing — over a real socket, on the native server.
///
/// The headline transaction-abort script, measured identically on 18.6:
/// `23505`, then `25P02` for every statement after it, then command tag `ROLLBACK`,
/// then no rows. The wasm bindings used to answer the `23505`, run the rest normally
/// and commit `1, 2, 3`.
///
/// **This is not a fixture because the differential harness cannot express the
/// class.** It propagates the first error from every statement it runs — `run_setup`
/// with `?`, `execute_test` by returning — so the statement that *creates* the
/// aborted state and the statement that *observes* it cannot sit on opposite sides of
/// the compared boundary: put the failure in `@setup` and the fixture's result is that
/// failure, put it in `@test` and nothing after it runs. That is a second, independent
/// blindness from the recorded one (every fixture runs inside `BEGIN … ROLLBACK`,
/// which restores the snapshot), and it is a property of the instrument rather than
/// evidence about the engine. The in-process guards live in `crates/pglet` and
/// `pglet_server`; this one is here because it is the only place a *real client over a
/// real socket* drives the server.
///
/// The `ROLLBACK` control beside it is the other polarity: a block whose statements
/// all succeed must still commit, or a session that aborted too eagerly would satisfy
/// the first half and lose every write in the engine.
async fn test_aborted_block_commits_nothing() -> Result<(), Box<dyn std::error::Error>> {
    let client = connect().await;

    exec(&client, "DROP TABLE IF EXISTS aborted_block").await;
    exec(&client, "CREATE TABLE aborted_block (id INT PRIMARY KEY)").await;

    exec(&client, "BEGIN").await;
    exec(&client, "INSERT INTO aborted_block VALUES (1)").await;
    exec(&client, "INSERT INTO aborted_block VALUES (2)").await;

    // The duplicate aborts the block.
    let dup = client
        .simple_query("INSERT INTO aborted_block VALUES (1)")
        .await
        .expect_err("a duplicate key must raise");
    assert_eq!(
        dup.code().map(tokio_postgres::error::SqlState::code),
        Some("23505"),
        "expected a unique violation, got {dup:?}"
    );

    // Everything after it is `25P02` — a read and a write alike.
    for sql in ["SELECT 1", "INSERT INTO aborted_block VALUES (3)"] {
        let e = client
            .simple_query(sql)
            .await
            .expect_err("an aborted block must refuse this");
        assert_eq!(
            e.code().map(tokio_postgres::error::SqlState::code),
            Some("25P02"),
            "{sql}: expected 25P02, got {e:?}"
        );
    }

    // The `COMMIT` is Postgres's implicit rollback: it succeeds, and reports
    // `ROLLBACK`.
    let tags: Vec<String> = client
        .simple_query("COMMIT")
        .await
        .expect("COMMIT in an aborted block must not error")
        .iter()
        .filter_map(|m| match m {
            tokio_postgres::SimpleQueryMessage::CommandComplete(_) => Some("done".to_owned()),
            _ => None,
        })
        .collect();
    assert_eq!(tags.len(), 1, "one CommandComplete for the COMMIT");

    assert_eq!(
        count(&client, "aborted_block").await,
        0,
        "the aborted block must publish nothing"
    );

    // The other polarity: a healthy block still commits everything it wrote.
    exec(&client, "BEGIN").await;
    exec(&client, "INSERT INTO aborted_block VALUES (7)").await;
    exec(&client, "COMMIT").await;
    assert_eq!(
        count(&client, "aborted_block").await,
        1,
        "a block with no failure must still commit"
    );

    exec(&client, "DROP TABLE aborted_block").await;

    println!("✓ Aborted block commits nothing");
    Ok(())
}

/// A comment-prefixed exit ends an aborted block instead of wedging the session.
///
/// The classifier reads the lexer, so the comment is stripped before anything
/// classifies. Under the `starts_with` text match all four arrangements answered
/// `25P02` **and so did every statement after them, `BEGIN` included**, until a bare
/// exit arrived; 18.6 ends the block on all of them. Driven over a real socket
/// because "the session is usable again" is the half an in-process gate test can
/// state but a client is what actually pays for.
async fn test_comment_prefixed_exit_unwedges_the_session() -> Result<(), Box<dyn std::error::Error>>
{
    let client = connect().await;

    exec(&client, "DROP TABLE IF EXISTS comment_exit").await;
    exec(&client, "CREATE TABLE comment_exit (id INT PRIMARY KEY)").await;

    for exit in [
        "/* trace */ COMMIT",
        "-- trace\nCOMMIT",
        "/* trace */ ROLLBACK",
        "-- trace\nROLLBACK",
    ] {
        exec(&client, "BEGIN").await;
        exec(&client, "INSERT INTO comment_exit VALUES (1)").await;
        let _ = client
            .simple_query("SELECT * FROM no_such_table_here")
            .await
            .expect_err("the block must abort");

        assert!(
            client.simple_query(exit).await.is_ok(),
            "{exit:?} must end the block"
        );

        // Usable, not wedged: an ordinary statement answers and a fresh block opens.
        assert!(
            client.simple_query("SELECT 1").await.is_ok(),
            "{exit:?} left the session wedged"
        );
        assert_eq!(
            count(&client, "comment_exit").await,
            0,
            "{exit:?}: the aborted block must publish nothing"
        );
    }

    exec(&client, "DROP TABLE comment_exit").await;

    println!("✓ Comment-prefixed exit unwedges the session");
    Ok(())
}

async fn test_rollback_visibility() -> Result<(), Box<dyn std::error::Error>> {
    let client1 = connect().await;
    let client2 = connect().await;

    // Setup
    exec(&client1, "DROP TABLE IF EXISTS rollback_test").await;
    exec(
        &client1,
        "CREATE TABLE rollback_test (id INT PRIMARY KEY, value INT)",
    )
    .await;
    exec(&client1, "INSERT INTO rollback_test VALUES (1, 100)").await;

    // Client 1 starts transaction and updates
    exec(&client1, "BEGIN").await;
    exec(
        &client1,
        "UPDATE rollback_test SET value = 200 WHERE id = 1",
    )
    .await;

    // Client 1 rolls back
    exec(&client1, "ROLLBACK").await;

    // Both clients should see original value
    let v1: i32 = query_one_i32(&client1, "SELECT value FROM rollback_test WHERE id = 1").await;
    let v2: i32 = query_one_i32(&client2, "SELECT value FROM rollback_test WHERE id = 1").await;
    assert_eq!(v1, 100, "Client 1 should see original after rollback");
    assert_eq!(
        v2, 100,
        "Client 2 should see original (rollback never visible)"
    );

    // Cleanup
    exec(&client1, "DROP TABLE rollback_test").await;

    println!("✓ Rollback visibility");
    Ok(())
}

// ============================================================================
// Test: Concurrent Inserts (Single-Writer Serialization)
// ============================================================================

async fn test_concurrent_inserts() -> Result<(), Box<dyn std::error::Error>> {
    let setup_client = connect().await;

    // Setup
    exec(&setup_client, "DROP TABLE IF EXISTS concurrent_test").await;
    exec(
        &setup_client,
        "CREATE TABLE concurrent_test (id INT PRIMARY KEY, value INT)",
    )
    .await;

    // Spawn multiple concurrent insert tasks
    let handles: Vec<_> = (0..5)
        .map(|i| {
            tokio::spawn(async move {
                let client = connect().await;
                exec(&client, "BEGIN").await;
                let sql = format!("INSERT INTO concurrent_test VALUES ({}, {})", i, i * 10);
                exec(&client, &sql).await;
                exec(&client, "COMMIT").await;
            })
        })
        .collect();

    // Wait for all to complete
    for h in handles {
        h.await?;
    }

    // Verify all 5 rows were inserted
    let cnt = count(&setup_client, "concurrent_test").await;
    assert_eq!(cnt, 5, "All 5 concurrent inserts should succeed");

    // Cleanup
    exec(&setup_client, "DROP TABLE concurrent_test").await;

    println!("✓ Concurrent inserts (five connections writing at once)");
    Ok(())
}

// ============================================================================
// Test: Connection Drop Rollback
// ============================================================================

async fn test_connection_drop_rollback() -> Result<(), Box<dyn std::error::Error>> {
    let persistent_client = connect().await;

    // Setup
    exec(&persistent_client, "DROP TABLE IF EXISTS drop_test").await;
    exec(
        &persistent_client,
        "CREATE TABLE drop_test (id INT PRIMARY KEY, value INT)",
    )
    .await;
    exec(&persistent_client, "INSERT INTO drop_test VALUES (1, 100)").await;

    // Create a client, start transaction, make changes, then drop without commit
    {
        let temp_client = connect().await;
        exec(&temp_client, "BEGIN").await;
        exec(
            &temp_client,
            "UPDATE drop_test SET value = 999 WHERE id = 1",
        )
        .await;
        // temp_client dropped here without COMMIT
    }

    // Give server time to detect disconnection and rollback
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Persistent client should see original value (transaction was rolled back)
    let value: i32 = query_one_i32(
        &persistent_client,
        "SELECT value FROM drop_test WHERE id = 1",
    )
    .await;
    assert_eq!(
        value, 100,
        "Dropped connection should rollback, value should be 100 not 999"
    );

    // Cleanup
    exec(&persistent_client, "DROP TABLE drop_test").await;

    println!("✓ Connection drop rollback");
    Ok(())
}

// ============================================================================
// Test: Multiple Readers During Write
// ============================================================================

async fn test_multiple_readers_during_write() -> Result<(), Box<dyn std::error::Error>> {
    let writer = connect().await;

    // Setup
    exec(&writer, "DROP TABLE IF EXISTS readers_test").await;
    exec(
        &writer,
        "CREATE TABLE readers_test (id INT PRIMARY KEY, value INT)",
    )
    .await;
    exec(&writer, "INSERT INTO readers_test VALUES (1, 100)").await;

    // Writer starts long transaction
    exec(&writer, "BEGIN").await;
    exec(&writer, "UPDATE readers_test SET value = 200 WHERE id = 1").await;

    // Spawn multiple concurrent readers
    let reader_handles: Vec<_> = (0..3)
        .map(|_| {
            tokio::spawn(async move {
                let reader = connect().await;
                let value: i32 =
                    query_one_i32(&reader, "SELECT value FROM readers_test WHERE id = 1").await;
                value
            })
        })
        .collect();

    // All readers should see original value (writer uncommitted)
    for h in reader_handles {
        let value = h.await?;
        assert_eq!(
            value, 100,
            "Reader should see original value during uncommitted write"
        );
    }

    exec(&writer, "COMMIT").await;

    // New reader sees committed value
    let final_reader = connect().await;
    let value: i32 =
        query_one_i32(&final_reader, "SELECT value FROM readers_test WHERE id = 1").await;
    assert_eq!(value, 200, "Reader after commit should see new value");

    // Cleanup
    exec(&writer, "DROP TABLE readers_test").await;

    println!("✓ Multiple readers during write");
    Ok(())
}

// ============================================================================
// Test: Stress Test (Many Concurrent Connections)
// ============================================================================

async fn test_stress_many_connections() -> Result<(), Box<dyn std::error::Error>> {
    let setup_client = connect().await;

    // Setup
    exec(&setup_client, "DROP TABLE IF EXISTS stress_test").await;
    exec(
        &setup_client,
        "CREATE TABLE stress_test (id INT PRIMARY KEY, value INT)",
    )
    .await;

    // Spawn 20 concurrent tasks, each doing a transaction
    let num_tasks: i64 = 20;
    let handles: Vec<_> = (0..num_tasks)
        .map(|i| {
            tokio::spawn(async move {
                let client = connect().await;
                exec(&client, "BEGIN").await;
                let sql = format!("INSERT INTO stress_test VALUES ({}, {})", i, i * 100);
                exec(&client, &sql).await;
                // Small delay to increase contention window
                tokio::time::sleep(Duration::from_millis(5)).await;
                exec(&client, "COMMIT").await;
            })
        })
        .collect();

    // Wait for all to complete
    for h in handles {
        h.await?;
    }

    // Verify all rows were inserted
    let cnt = count(&setup_client, "stress_test").await;
    assert_eq!(
        cnt, num_tasks,
        "All {num_tasks} concurrent inserts should succeed"
    );

    // Cleanup
    exec(&setup_client, "DROP TABLE stress_test").await;

    println!("✓ Stress test ({num_tasks} concurrent connections)");
    Ok(())
}

// ============================================================================
// Test: Concurrent DDL (CREATE/DROP during queries)
// ============================================================================

async fn test_concurrent_ddl() -> Result<(), Box<dyn std::error::Error>> {
    let setup_client = connect().await;

    // Setup base table
    exec(&setup_client, "DROP TABLE IF EXISTS ddl_test").await;
    exec(
        &setup_client,
        "CREATE TABLE ddl_test (id INT PRIMARY KEY, value INT)",
    )
    .await;
    exec(&setup_client, "INSERT INTO ddl_test VALUES (1, 100)").await;

    // Start a reader in a transaction (holds snapshot)
    let reader = connect().await;
    exec(&reader, "BEGIN").await;
    let initial: i32 = query_one_i32(&reader, "SELECT value FROM ddl_test WHERE id = 1").await;
    assert_eq!(initial, 100);

    // Another connection creates a new table while reader has open transaction
    let ddl_client = connect().await;
    exec(&ddl_client, "CREATE TABLE ddl_new (x INT)").await;
    exec(&ddl_client, "INSERT INTO ddl_new VALUES (42)").await;

    // Reader should still be able to query (its snapshot is consistent)
    let during: i32 = query_one_i32(&reader, "SELECT value FROM ddl_test WHERE id = 1").await;
    assert_eq!(during, 100);

    // Reader commits, now can see new table
    exec(&reader, "COMMIT").await;
    let new_val: i32 = query_one_i32(&reader, "SELECT x FROM ddl_new").await;
    assert_eq!(new_val, 42);

    // Cleanup
    exec(&setup_client, "DROP TABLE ddl_test").await;
    exec(&setup_client, "DROP TABLE ddl_new").await;

    println!("✓ Concurrent DDL");
    Ok(())
}

// ============================================================================
// Test: Concurrent committed DDL vs another txn's ROLLBACK
// ============================================================================

async fn test_concurrent_ddl_rollback_no_clobber() -> Result<(), Box<dyn std::error::Error>> {
    // A block's ROLLBACK must not roll the shared catalog back over a table another
    // connection committed. With the writer, B's CREATE TABLE waits for A's block —
    // which holds it from its INSERT — and runs after A's rollback, so it cannot be
    // clobbered by it; the second half is B's DDL committed while A's block has only
    // read, which A's rollback restores nothing over. (Before the writer, A's rollback
    // restored `name_to_id`/`next_table_id` and the catalog wholesale over B's table.)
    let a = connect().await;
    let b = connect_patient().await;

    for a_writes_first in [true, false] {
        let label = if a_writes_first {
            "B waits for A's written block"
        } else {
            "B commits while A has only read"
        };
        exec(&a, "DROP TABLE IF EXISTS clobber_base").await;
        exec(&a, "DROP TABLE IF EXISTS clobber_concurrent").await;
        exec(&a, "DROP TABLE IF EXISTS clobber_after").await;
        exec(&a, "CREATE TABLE clobber_base (id INT)").await;
        exec(&a, "INSERT INTO clobber_base VALUES (1)").await;

        exec(&a, "BEGIN").await;
        if a_writes_first {
            exec(&a, "INSERT INTO clobber_base VALUES (2)").await;
            let task = spawn_statements(
                &b,
                &[
                    "CREATE TABLE clobber_concurrent (x INT)",
                    "INSERT INTO clobber_concurrent VALUES (42)",
                ],
            );
            let codes = after_holder_ends(&a, "ROLLBACK", task, label).await;
            assert_eq!(codes, vec![None, None], "{label}");
        } else {
            assert_eq!(count(&a, "clobber_base").await, 1);
            exec(&b, "CREATE TABLE clobber_concurrent (x INT)").await;
            exec(&b, "INSERT INTO clobber_concurrent VALUES (42)").await;
            exec(&a, "ROLLBACK").await;
        }

        let from_b: i32 = query_one_i32(&b, "SELECT x FROM clobber_concurrent").await;
        assert_eq!(from_b, 42, "{label}: B's committed table survives");
        let from_a: i32 = query_one_i32(&a, "SELECT x FROM clobber_concurrent").await;
        assert_eq!(from_a, 42, "{label}: A sees it after its rollback");
        assert_eq!(
            count(&a, "clobber_base").await,
            1,
            "{label}: A's insert rolled back"
        );

        // No id collision: a fresh table allocates a distinct id and is usable.
        exec(&a, "CREATE TABLE clobber_after (y INT)").await;
        exec(&a, "INSERT INTO clobber_after VALUES (7)").await;
        let after: i32 = query_one_i32(&a, "SELECT y FROM clobber_after").await;
        assert_eq!(after, 7, "{label}");
        let still: i32 = query_one_i32(&a, "SELECT x FROM clobber_concurrent").await;
        assert_eq!(still, 42, "{label}: not aliased by a reused id");
    }

    exec(&a, "DROP TABLE clobber_base").await;
    exec(&a, "DROP TABLE clobber_concurrent").await;
    exec(&a, "DROP TABLE clobber_after").await;

    println!("✓ Concurrent committed DDL survives another txn's rollback");
    Ok(())
}

async fn test_concurrent_table_vs_index_ddl_rollback_no_divergence()
-> Result<(), Box<dyn std::error::Error>> {
    // A block whose own DDL is a CREATE INDEX, and another connection's CREATE TABLE:
    // B's DDL waits for A's block and runs after A's ROLLBACK, which restores the
    // catalog and storage whole (the index goes) and leaves B's table to be created —
    // one catalog, one storage, both holding it. (Before the writer, A's rollback met B's
    // committed table and refused `22000` rather than clobber it.)
    let a = connect().await;
    let b = connect_patient().await;

    exec(&a, "DROP TABLE IF EXISTS idx_base").await;
    exec(&a, "DROP TABLE IF EXISTS idx_concurrent").await;
    exec(&a, "CREATE TABLE idx_base (id INT, v INT)").await;
    exec(&a, "INSERT INTO idx_base VALUES (1, 10)").await;

    exec(&a, "BEGIN").await;
    exec(&a, "CREATE INDEX idx_base_v ON idx_base(v)").await;
    let task = spawn_statements(
        &b,
        &[
            "CREATE TABLE idx_concurrent (x INT)",
            "INSERT INTO idx_concurrent VALUES (99)",
        ],
    );
    settle().await;
    assert!(!task.is_finished(), "B's DDL waits for A's block");
    assert_eq!(
        sqlstate(&a, "ROLLBACK").await,
        None,
        "A's rollback restores, no refusal"
    );
    assert_eq!(task.await?, vec![None, None]);

    let from_b: i32 = query_one_i32(&b, "SELECT x FROM idx_concurrent").await;
    assert_eq!(from_b, 99, "B's committed table");
    let from_a: i32 = query_one_i32(&a, "SELECT x FROM idx_concurrent").await;
    assert_eq!(
        from_a, 99,
        "A sees B's table — no catalog/storage divergence"
    );
    // A's index is gone: its name is free again.
    assert_eq!(
        sqlstate(&a, "CREATE INDEX idx_base_v ON idx_base(v)").await,
        None,
        "the rolled-back index left no phantom"
    );

    exec(&a, "DROP TABLE IF EXISTS idx_base").await;
    exec(&a, "DROP TABLE IF EXISTS idx_concurrent").await;

    println!("✓ Concurrent table vs index DDL rollback (no divergence)");
    Ok(())
}

async fn test_drop_in_txn_rollback_restores_committed_rows()
-> Result<(), Box<dyn std::error::Error>> {
    // Single-connection DROP-in-transaction-rollback must restore the table's
    // committed rows (the row-loss residual). The in-transaction DROP (and its
    // index cascade) is buffered — deferred to commit — so ROLLBACK leaves the rows,
    // SYS_TABLE, and index entries untouched while the server restores the catalog
    // from its BEGIN snapshot.
    let a = connect().await;

    exec(&a, "DROP TABLE IF EXISTS drop_rollback_t").await;
    exec(&a, "CREATE TABLE drop_rollback_t (id INT, v INT)").await;
    exec(&a, "INSERT INTO drop_rollback_t VALUES (1, 10)").await;
    exec(&a, "INSERT INTO drop_rollback_t VALUES (2, 20)").await;
    exec(&a, "CREATE INDEX drop_rollback_idx ON drop_rollback_t(v)").await;

    exec(&a, "BEGIN").await;
    exec(&a, "DROP TABLE drop_rollback_t").await;
    exec(&a, "ROLLBACK").await;

    // The table and BOTH committed rows are back.
    let n = count(&a, "drop_rollback_t").await;
    assert_eq!(n, 2, "rolled-back DROP must restore all committed rows");
    let v: i32 = query_one_i32(&a, "SELECT v FROM drop_rollback_t WHERE id = 2").await;
    assert_eq!(v, 20, "restored rows must retain their values");
    // The cascade-dropped index is restored too — the probe still resolves.
    let id: i32 = query_one_i32(&a, "SELECT id FROM drop_rollback_t WHERE v = 10").await;
    assert_eq!(
        id, 1,
        "the index dropped by the cascade must be restored on rollback"
    );

    exec(&a, "DROP TABLE drop_rollback_t").await;
    println!("✓ DROP-in-txn rollback restores committed rows + index");
    Ok(())
}

async fn test_create_index_in_txn_rollback_no_phantom() -> Result<(), Box<dyn std::error::Error>> {
    // CREATE INDEX inside a txn is built immediately (read-your-own-writes), so its
    // ROLLBACK undoes it at the storage layer (deregister def + shed band) under a
    // fresh apply_batch. End-to-end smoke through the real server: the connection
    // stays usable, the index is gone from the catalog (re-creating the same name
    // succeeds rather than erroring "already exists"), and queries stay correct after
    // the band was shed and rebuilt. (The phantom itself is catalog-invisible; the
    // storage unit tests assert no lingering def / id-reuse aliasing directly.)
    let a = connect().await;

    exec(&a, "DROP TABLE IF EXISTS ci_txn").await;
    exec(&a, "CREATE TABLE ci_txn (id INT, v INT)").await;
    exec(&a, "INSERT INTO ci_txn VALUES (1, 10)").await;
    exec(&a, "INSERT INTO ci_txn VALUES (2, 20)").await;

    exec(&a, "BEGIN").await;
    exec(&a, "CREATE INDEX ci_txn_v ON ci_txn(v)").await;
    exec(&a, "ROLLBACK").await;

    // Connection usable + the index rolled out of the catalog: re-creating the same
    // name must succeed (it would error if the rolled-back index lingered).
    a.simple_query("CREATE INDEX ci_txn_v ON ci_txn(v)")
        .await
        .expect("re-CREATE INDEX after rollback must succeed");

    // The rebuilt index returns correct results (no stale phantom entries).
    let n: i64 = count(&a, "ci_txn").await;
    assert_eq!(n, 2);
    let id: i32 = query_one_i32(&a, "SELECT id FROM ci_txn WHERE v = 20").await;
    assert_eq!(id, 2, "the rebuilt index must probe correctly");

    exec(&a, "DROP TABLE IF EXISTS ci_txn").await;
    println!("✓ CREATE INDEX in txn rollback leaves no lingering index");
    Ok(())
}

async fn test_do_block_ddl_rolls_back() -> Result<(), Box<dyn std::error::Error>> {
    // A `DO $$…$$` block that runs DDL must roll back with its transaction. The block
    // re-enters the executor for each inner statement, so one top-level `DO` statement
    // bumps `Schema::generation` more than once (the block's own bump plus each inner
    // DDL's); a rollback that counted DDL per statement would undershoot it and leave the
    // enum type behind — the cross-fixture pollution where, after
    // `ddl/do_block_create_type`, a later `CREATE TYPE cell_visibility` failed 42710.
    //
    // Covers a single DO block with *two* inner `CREATE TYPE`s (one statement bumps the
    // generation 3×), a swallowed `duplicate_object` replay (the failed inner DDL still
    // bumps the generation before execution), and a following plain `CREATE TABLE`
    // (accumulation across statements) — all rolled back together.
    let a = connect().await;

    exec(&a, "DROP TABLE IF EXISTS do_rb_t").await;
    exec(&a, "DROP TYPE IF EXISTS do_rb_vis").await;
    exec(&a, "DROP TYPE IF EXISTS do_rb_kind").await;

    exec(&a, "BEGIN").await;
    // one DO block, two inner CREATE TYPEs → this single statement bumps the generation
    // three times (block + two inner)
    a.simple_query(
        "DO $$ BEGIN \
            CREATE TYPE do_rb_vis AS ENUM ('private', 'public'); \
            CREATE TYPE do_rb_kind AS ENUM ('note', 'task'); \
         EXCEPTION WHEN duplicate_object THEN NULL; END $$",
    )
    .await
    .expect("multi-DDL DO block should run");
    // Idempotent replay: do_rb_vis already exists, so the inner CREATE TYPE raises
    // duplicate_object and is swallowed — but it still bumped the generation, so the
    // count must include it.
    a.simple_query(
        "DO $$ BEGIN CREATE TYPE do_rb_vis AS ENUM ('private', 'public'); \
         EXCEPTION WHEN duplicate_object THEN NULL; END $$",
    )
    .await
    .expect("idempotent replay DO block should run");
    a.simple_query(
        "CREATE TABLE do_rb_t (id INT, visibility do_rb_vis NOT NULL DEFAULT 'private')",
    )
    .await
    .expect("CREATE TABLE in the transaction should run");
    exec(&a, "ROLLBACK").await;

    // ROLLBACK restored the catalog: both enum types and the table are gone, so
    // re-creating each succeeds. Before the fix these failed 42710 / table-exists because
    // the leaked catalog objects lingered.
    a.simple_query("CREATE TYPE do_rb_vis AS ENUM ('private', 'public')")
        .await
        .expect("do_rb_vis must be gone after ROLLBACK — the DO-block DDL must roll back");
    a.simple_query("CREATE TYPE do_rb_kind AS ENUM ('note', 'task')")
        .await
        .expect("do_rb_kind (the block's second inner CREATE TYPE) must be gone after ROLLBACK");
    a.simple_query("CREATE TABLE do_rb_t (id INT, visibility do_rb_vis)")
        .await
        .expect("table must be gone after ROLLBACK");

    exec(&a, "DROP TABLE IF EXISTS do_rb_t").await;
    exec(&a, "DROP TYPE IF EXISTS do_rb_vis").await;
    exec(&a, "DROP TYPE IF EXISTS do_rb_kind").await;
    println!("✓ DO-block DDL rolls back with its transaction");
    Ok(())
}

async fn test_do_block_ddl_commits() -> Result<(), Box<dyn std::error::Error>> {
    // The complement of the rollback test: DO-block DDL inside an explicit transaction
    // must *persist* on COMMIT (the delta-tracking fix must not over-eagerly undo it),
    // and COMMIT must reset the per-transaction generation-delta count so a *later*
    // DML-only transaction's ROLLBACK doesn't mis-fire and clobber the committed catalog.
    let a = connect().await;

    exec(&a, "DROP TABLE IF EXISTS do_c_t").await;
    exec(&a, "DROP TYPE IF EXISTS do_c_vis").await;

    exec(&a, "BEGIN").await;
    a.simple_query(
        "DO $$ BEGIN CREATE TYPE do_c_vis AS ENUM ('private', 'public'); \
         EXCEPTION WHEN duplicate_object THEN NULL; END $$",
    )
    .await
    .expect("DO block should run");
    a.simple_query("CREATE TABLE do_c_t (id INT, visibility do_c_vis NOT NULL DEFAULT 'private')")
        .await
        .expect("CREATE TABLE in the transaction should run");
    exec(&a, "COMMIT").await;

    // The type persisted: re-creating it now fails (it really committed, not rolled back).
    let re_create = a
        .simple_query("CREATE TYPE do_c_vis AS ENUM ('private', 'public')")
        .await;
    assert!(
        re_create.is_err(),
        "committed DO-block CREATE TYPE must persist (re-create should fail duplicate)"
    );
    // The committed table is usable.
    exec(&a, "INSERT INTO do_c_t (id) VALUES (1)").await;
    let n = count(&a, "do_c_t").await;
    assert_eq!(n, 1, "committed DO-block-backed table must be usable");

    // A subsequent DML-only transaction's ROLLBACK must not clobber the committed catalog
    // (COMMIT reset the generation-delta count; a stale nonzero count would make this
    // rollback restore an older schema snapshot).
    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO do_c_t (id) VALUES (2)").await;
    exec(&a, "ROLLBACK").await;
    let after = count(&a, "do_c_t").await;
    assert_eq!(
        after, 1,
        "DML rollback must undo its own insert, not the committed schema"
    );
    let still: i32 = query_one_i32(&a, "SELECT id FROM do_c_t WHERE id = 1").await;
    assert_eq!(
        still, 1,
        "the committed type-backed table must survive a later DML rollback"
    );

    exec(&a, "DROP TABLE IF EXISTS do_c_t").await;
    exec(&a, "DROP TYPE IF EXISTS do_c_vis").await;
    println!("✓ DO-block DDL persists on COMMIT (and survives a later DML rollback)");
    Ok(())
}

// ============================================================================
// Test: Write Conflict (Multiple writers on same row)
// ============================================================================

async fn test_write_conflict() -> Result<(), Box<dyn std::error::Error>> {
    let setup_client = connect().await;

    // Setup
    exec(&setup_client, "DROP TABLE IF EXISTS conflict_test").await;
    exec(
        &setup_client,
        "CREATE TABLE conflict_test (id INT PRIMARY KEY, counter INT)",
    )
    .await;
    exec(&setup_client, "INSERT INTO conflict_test VALUES (1, 0)").await;

    // Spawn multiple writers trying to increment the same counter
    // With snapshot isolation, they each see the original value
    let num_writers = 5;
    let handles: Vec<_> = (0..num_writers)
        .map(|_| {
            tokio::spawn(async move {
                let client = connect().await;
                exec(&client, "BEGIN").await;
                // Read current value
                let current: i32 =
                    query_one_i32(&client, "SELECT counter FROM conflict_test WHERE id = 1").await;
                // Increment by 1
                let new_val = current + 1;
                let sql = format!("UPDATE conflict_test SET counter = {new_val} WHERE id = 1");
                exec(&client, &sql).await;
                exec(&client, "COMMIT").await;
            })
        })
        .collect();

    // Wait for all to complete
    for h in handles {
        h.await?;
    }

    // Each writer reads the counter before its first write and computes the new value
    // client-side, so writers that read before another's commit lose that commit's
    // increment — on 18.6 `READ COMMITTED` too. The writer serializes the UPDATEs (a
    // later one waits for the earlier block's COMMIT); it cannot restore an increment
    // the client computed from a stale read. The key invariant: no crash, no deadlock,
    // no timeout, value is consistent.
    let final_val: i32 = query_one_i32(
        &setup_client,
        "SELECT counter FROM conflict_test WHERE id = 1",
    )
    .await;

    assert!(
        final_val >= 1,
        "Counter should be at least 1, got {final_val}"
    );

    // Cleanup
    exec(&setup_client, "DROP TABLE conflict_test").await;

    println!("✓ Write conflict (client-side read-modify-write, no deadlock)");
    Ok(())
}

// ============================================================================
// Main runner
// ============================================================================

// ============================================================================
// The writer: at most one connection's block stages writes
//
// A block takes the writer at its first write and holds it until it ends; an
// autocommit write holds it for its statement. A second writer **waits** — bounded by
// its `lock_timeout` — and then answers against what the holder committed or took back,
// which is 18.6's answer for every shape below (each one's 18.6 answer is the comment
// beside it, measured against 18.6 over two connections). What diverges is only **when**
// a writer waits: pglet's writer serializes unrelated writes too, where 18.6 blocks only
// a conflicting row, table or index key.
//
// Every waiting statement runs in its own task with a long `lock_timeout`, so a loaded
// box cannot turn a wait into a `55P03`; the holder ends its block only after the
// statement has been seen still waiting.
//
// The suite shares one server, so each test owns its table and drops it.
// ============================================================================

/// A connection whose writes wait out a holder for up to ten seconds.
async fn connect_patient() -> std::sync::Arc<Client> {
    let client = connect().await;
    exec(&client, "SET lock_timeout = '10s'").await;
    std::sync::Arc::new(client)
}

/// Run `statements` in order on `client` in a task of their own, answering each one's
/// SQLSTATE (`None` for success) once it has run — the first may wait for the writer.
fn spawn_statements(
    client: &std::sync::Arc<Client>,
    statements: &[&str],
) -> tokio::task::JoinHandle<Vec<Option<String>>> {
    let client = std::sync::Arc::clone(client);
    let statements: Vec<String> = statements.iter().map(|s| (*s).to_string()).collect();
    tokio::spawn(async move {
        let mut codes = Vec::new();
        for sql in statements {
            codes.push(sqlstate(&client, &sql).await);
        }
        codes
    })
}

/// Long enough that a statement answering at once has answered.
async fn settle() {
    tokio::time::sleep(Duration::from_millis(300)).await;
}

/// Assert `task` is still waiting, then end the holder's block with `end`, then answer
/// what the task's statements answered.
async fn after_holder_ends(
    holder: &Client,
    end: &str,
    task: tokio::task::JoinHandle<Vec<Option<String>>>,
    label: &str,
) -> Vec<Option<String>> {
    settle().await;
    assert!(
        !task.is_finished(),
        "{label}: the statement waits for the holder's block"
    );
    exec(holder, end).await;
    task.await.expect("the waiting task")
}

/// The narrowest shape: a key staged in an open block, and a second connection's
/// insert of it. The second writer waits for the block and then answers against its
/// outcome — `23505` when it commits, an insert when it rolls back — which is 18.6's
/// answer both ways. The rolled-back half was pglet's one loud residual of the
/// answer-at-once design (a refusal that stood although the holder rolled back).
async fn test_second_writer_waits_for_the_holder() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;
    for (end, code, rows) in [
        ("COMMIT", Some("23505"), vec!["dup|1"]),
        ("ROLLBACK", None, vec!["dup|2"]),
    ] {
        exec(&a, "DROP TABLE IF EXISTS w_pk").await;
        exec(&a, "CREATE TABLE w_pk (a TEXT PRIMARY KEY, v INT)").await;
        exec(&a, "BEGIN").await;
        exec(&a, "INSERT INTO w_pk VALUES ('dup', 1)").await;
        let task = spawn_statements(&b, &["INSERT INTO w_pk VALUES ('dup', 2)"]);
        let codes = after_holder_ends(&a, end, task, end).await;
        assert_eq!(
            codes[0].as_deref(),
            code,
            "{end}: the second writer's answer"
        );
        assert_eq!(
            text_rows(&a, "SELECT a || '|' || v FROM w_pk ORDER BY 1").await,
            rows,
            "{end}"
        );
    }
    exec(&a, "DROP TABLE w_pk").await;
    println!("✓ a second writer waits for the holder, then answers its outcome");
    Ok(())
}

/// The control: nothing about the writer may change what one connection sees of
/// **itself** or of another connection's **committed** rows.
///
/// Two halves, and the second is the one a writer that waited on its own block fails:
/// a block that stages a key and then probes that same key must see its own row.
async fn test_committed_and_own_key_conflicts_unchanged() -> Result<(), Box<dyn std::error::Error>>
{
    let a = connect().await;
    let b = connect().await;

    exec(&a, "DROP TABLE IF EXISTS claim_control").await;
    exec(&a, "CREATE TABLE claim_control (a TEXT PRIMARY KEY, v INT)").await;

    // Half one: an ordinary committed-row conflict across two connections.
    assert_eq!(
        sqlstate(&a, "INSERT INTO claim_control VALUES ('c', 1)").await,
        None,
        "an uncontended insert must still succeed"
    );
    assert_eq!(
        sqlstate(&b, "INSERT INTO claim_control VALUES ('c', 2)")
            .await
            .as_deref(),
        Some("23505"),
        "a committed row must still conflict"
    );

    // Half two: read-your-own-writes. A stages a key and then arbitrates against it in
    // the same block — the hit is its *own*, so `DO NOTHING` skips and the block
    // commits one row rather than being refused by itself.
    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO claim_control VALUES ('own', 1)").await;
    assert_eq!(
        sqlstate(
            &a,
            "INSERT INTO claim_control VALUES ('own', 2) ON CONFLICT (a) DO NOTHING"
        )
        .await,
        None,
        "a connection must not conflict with itself"
    );
    exec(&a, "UPDATE claim_control SET v = v + 1 WHERE a = 'own'").await;
    assert_eq!(
        sqlstate(&a, "UPDATE claim_control SET v = v + 1 WHERE a = 'own'").await,
        None,
        "a row updating itself must not be refused by its own staged write"
    );
    exec(&a, "COMMIT").await;

    let rows = text_rows(&a, "SELECT a || '|' || v FROM claim_control ORDER BY 1").await;
    assert_eq!(rows, vec!["c|1", "own|3"]);

    exec(&a, "DROP TABLE claim_control").await;
    println!("✓ committed-row and own-key conflicts unchanged");
    Ok(())
}

/// Every `ON CONFLICT` action over a key another connection's block staged waits, then
/// acts on the row that became visible — or, if the holder rolled back, on no row: a
/// skip or an update after a `COMMIT`, an insert after a `ROLLBACK`. 18.6's answers,
/// both outcomes, both `DO NOTHING` spellings. (The answer-at-once design refused all of
/// these `23505`, since without the wait no answer was right under both outcomes.)
async fn test_on_conflict_actions_after_the_wait() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;
    for (label, stmt, committed, rolled_back) in [
        (
            "target-less DO NOTHING",
            "INSERT INTO w_oc VALUES ('k', 2) ON CONFLICT DO NOTHING",
            "k|1",
            "k|2",
        ),
        (
            "arbitrated DO NOTHING",
            "INSERT INTO w_oc VALUES ('k', 3) ON CONFLICT (a) DO NOTHING",
            "k|1",
            "k|3",
        ),
        (
            "DO UPDATE",
            "INSERT INTO w_oc VALUES ('k', 4) ON CONFLICT (a) DO UPDATE SET v = EXCLUDED.v",
            "k|4",
            "k|4",
        ),
    ] {
        for (end, rows) in [("COMMIT", committed), ("ROLLBACK", rolled_back)] {
            let cell = format!("{label}, the holder's {end}");
            exec(&a, "DROP TABLE IF EXISTS w_oc").await;
            exec(&a, "CREATE TABLE w_oc (a TEXT PRIMARY KEY, v INT)").await;
            exec(&a, "BEGIN").await;
            exec(&a, "INSERT INTO w_oc VALUES ('k', 1)").await;
            let task = spawn_statements(&b, &[stmt]);
            let codes = after_holder_ends(&a, end, task, &cell).await;
            assert_eq!(codes[0], None, "{cell}");
            assert_eq!(
                text_rows(&a, "SELECT a || '|' || v FROM w_oc ORDER BY 1").await,
                vec![rows],
                "{cell}"
            );
        }
    }
    exec(&a, "DROP TABLE w_oc").await;
    println!("✓ ON CONFLICT actions after the wait (both outcomes)");
    Ok(())
}

/// A target-less `DO NOTHING` over a table with two unique keys, one of them staged by
/// another connection's block over a committed row: 18.6 pre-checks every unique index,
/// and the claim it waits on can only confirm a conflict or vanish — `(1,200)` and
/// `(2,100)` skip under both holder outcomes, `(2,300)` skips after a `COMMIT` and
/// inserts after a `ROLLBACK`, and an uncontended `(4,400)` inserts. The rows are the
/// ones 18.6 ends with.
async fn test_do_nothing_beside_a_staged_key_after_the_wait()
-> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;
    for (end, rows) in [
        ("COMMIT", vec!["1|100", "2|200", "4|400"]),
        ("ROLLBACK", vec!["1|100", "2|300", "4|400"]),
    ] {
        exec(&a, "DROP TABLE IF EXISTS w_two").await;
        exec(&a, "CREATE TABLE w_two (a INT UNIQUE, b INT UNIQUE)").await;
        exec(&a, "INSERT INTO w_two VALUES (1, 100)").await;
        exec(&a, "BEGIN").await;
        exec(&a, "INSERT INTO w_two VALUES (2, 200)").await;
        let task = spawn_statements(
            &b,
            &[
                "INSERT INTO w_two VALUES (1, 200) ON CONFLICT DO NOTHING",
                "INSERT INTO w_two VALUES (2, 100) ON CONFLICT DO NOTHING",
                "INSERT INTO w_two VALUES (2, 300) ON CONFLICT DO NOTHING",
                "INSERT INTO w_two VALUES (4, 400) ON CONFLICT DO NOTHING",
            ],
        );
        let codes = after_holder_ends(&a, end, task, end).await;
        assert_eq!(codes, vec![None, None, None, None], "{end}");
        assert_eq!(
            text_rows(&a, "SELECT a || '|' || b FROM w_two ORDER BY 1").await,
            rows,
            "{end}"
        );
    }
    exec(&a, "DROP TABLE w_two").await;
    println!("✓ DO NOTHING beside a staged key, after the wait");
    Ok(())
}

/// What a staged row would index decides the second writer's answer once the holder
/// commits: a plain `UNIQUE` key conflicts, a partial unique index's predicate decides
/// membership, `NULLS NOT DISTINCT` makes two NULLs collide, and under the default a
/// NULL is no uniqueness candidate — 18.6's answers after its wait. And a row staged by
/// `DELETE` then `INSERT` outside the predicate leaves the key free after a `ROLLBACK`.
async fn test_index_shapes_after_the_wait() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;

    for table in ["w_un", "w_pu", "w_nn", "w_nd"] {
        exec(&a, &format!("DROP TABLE IF EXISTS {table}")).await;
    }
    exec(&a, "CREATE TABLE w_un (id INT, u TEXT UNIQUE)").await;
    exec(&a, "CREATE TABLE w_pu (a TEXT, live BOOL)").await;
    exec(&a, "CREATE UNIQUE INDEX w_pu_a ON w_pu (a) WHERE live").await;
    exec(&a, "CREATE TABLE w_nn (a TEXT, v INT)").await;
    exec(
        &a,
        "CREATE UNIQUE INDEX w_nn_a ON w_nn (a) NULLS NOT DISTINCT",
    )
    .await;
    exec(&a, "CREATE TABLE w_nd (a TEXT UNIQUE, v INT)").await;

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_un VALUES (1, 'x')").await;
    exec(&a, "INSERT INTO w_pu VALUES ('k', true)").await;
    exec(&a, "INSERT INTO w_nn VALUES (NULL, 1)").await;
    exec(&a, "INSERT INTO w_nd VALUES (NULL, 1)").await;
    let task = spawn_statements(
        &b,
        &[
            "INSERT INTO w_un VALUES (2, 'x')",
            "INSERT INTO w_pu VALUES ('k', false)",
            "INSERT INTO w_pu VALUES ('k', true)",
            "INSERT INTO w_nn VALUES (NULL, 2)",
            "INSERT INTO w_nd VALUES (NULL, 2)",
        ],
    );
    let codes = after_holder_ends(&a, "COMMIT", task, "index shapes").await;
    assert_eq!(
        codes.iter().map(Option::as_deref).collect::<Vec<_>>(),
        vec![Some("23505"), None, Some("23505"), Some("23505"), None],
        "UNIQUE, partial excluded, partial included, NULLS NOT DISTINCT, NULLS DISTINCT"
    );
    assert_eq!(count(&a, "w_un").await, 1);
    assert_eq!(
        text_rows(&a, "SELECT a || '|' || live FROM w_pu ORDER BY 1").await,
        vec!["k|false", "k|true"],
    );
    assert_eq!(count(&a, "w_nn").await, 1);
    assert_eq!(count(&a, "w_nd").await, 2);

    exec(&a, "INSERT INTO w_pu VALUES ('p', false)").await;
    exec(&a, "BEGIN").await;
    exec(&a, "DELETE FROM w_pu WHERE a = 'p'").await;
    exec(&a, "INSERT INTO w_pu VALUES ('p', false)").await;
    let task = spawn_statements(&b, &["INSERT INTO w_pu VALUES ('p', true)"]);
    let codes = after_holder_ends(&a, "ROLLBACK", task, "restaged outside the predicate").await;
    assert_eq!(codes, vec![None]);

    for table in ["w_un", "w_pu", "w_nn", "w_nd"] {
        exec(&a, &format!("DROP TABLE {table}")).await;
    }
    println!("✓ UNIQUE, partial and NULLS NOT DISTINCT shapes, after the wait");
    Ok(())
}

/// Column DDL inside the holder's block — the write the pending index cannot follow — and
/// a second writer after it. 18.6 holds ACCESS EXCLUSIVE for the uncommitted `ALTER
/// TABLE`, so its probe waits for the block; after a `COMMIT` it answers `23505` for a
/// row the holder updated into the partial predicate after the DDL, an insert for a key
/// only a predicate-excluded row holds, and an insert for a key whose staged row the
/// holder deleted after the DDL. pglet's writer waits the same, and answers the same.
async fn test_column_ddl_then_a_second_writer() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;

    exec(&a, "DROP TABLE IF EXISTS w_scan").await;
    exec(&a, "CREATE TABLE w_scan (a TEXT, live BOOL)").await;
    exec(&a, "CREATE UNIQUE INDEX w_scan_a ON w_scan (a) WHERE live").await;

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_scan (a, live) VALUES ('k', false)").await;
    exec(&a, "ALTER TABLE w_scan ADD COLUMN w INT").await;
    exec(&a, "UPDATE w_scan SET live = true WHERE a = 'k'").await;
    exec(&a, "INSERT INTO w_scan (a, live) VALUES ('p', false)").await;
    let task = spawn_statements(
        &b,
        &[
            "INSERT INTO w_scan (a, live) VALUES ('k', true)",
            "INSERT INTO w_scan (a, live) VALUES ('p', true)",
        ],
    );
    let codes = after_holder_ends(&a, "COMMIT", task, "half one").await;
    assert_eq!(
        codes.iter().map(Option::as_deref).collect::<Vec<_>>(),
        vec![Some("23505"), None]
    );

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_scan (a, live) VALUES ('d', true)").await;
    exec(&a, "ALTER TABLE w_scan ADD COLUMN w2 INT").await;
    exec(&a, "DELETE FROM w_scan WHERE a = 'd'").await;
    let task = spawn_statements(&b, &["INSERT INTO w_scan (a, live) VALUES ('d', true)"]);
    let codes = after_holder_ends(&a, "COMMIT", task, "half two").await;
    assert_eq!(codes, vec![None]);

    assert_eq!(
        text_rows(&a, "SELECT a || '|' || live FROM w_scan ORDER BY 1").await,
        vec!["d|true", "k|true", "p|false", "p|true"],
    );
    exec(&a, "DROP TABLE w_scan").await;
    println!("✓ column DDL then a second writer");
    Ok(())
}

/// An `UPDATE` moving a row onto a key another connection's block staged — by an insert
/// or by an update — waits and then answers `23505` once the holder commits; and a key
/// the holder's update moved a staged row **off** is free, an insert. 18.6's answers
/// after its wait, measured with the holder committing each time.
async fn test_update_onto_a_staged_key_after_the_wait() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;

    exec(&a, "DROP TABLE IF EXISTS w_upd").await;
    exec(&a, "CREATE TABLE w_upd (a TEXT PRIMARY KEY, v INT)").await;
    exec(&a, "INSERT INTO w_upd VALUES ('other', 9)").await;

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_upd VALUES ('k', 1)").await;
    let task = spawn_statements(&b, &["UPDATE w_upd SET a = 'k' WHERE a = 'other'"]);
    let codes = after_holder_ends(&a, "COMMIT", task, "onto an inserted key").await;
    assert_eq!(codes[0].as_deref(), Some("23505"));
    assert_eq!(
        text_rows(&a, "SELECT a || '|' || v FROM w_upd ORDER BY 1").await,
        vec!["k|1", "other|9"],
    );

    exec(&a, "BEGIN").await;
    exec(&a, "UPDATE w_upd SET a = 'moved' WHERE a = 'other'").await;
    let task = spawn_statements(&b, &["INSERT INTO w_upd VALUES ('moved', 5)"]);
    let codes = after_holder_ends(&a, "COMMIT", task, "onto an updated key").await;
    assert_eq!(codes[0].as_deref(), Some("23505"));
    assert_eq!(
        text_rows(&a, "SELECT a || '|' || v FROM w_upd ORDER BY 1").await,
        vec!["k|1", "moved|9"],
    );

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_upd VALUES ('staged', 1)").await;
    exec(&a, "UPDATE w_upd SET a = 'restaged' WHERE a = 'staged'").await;
    let task = spawn_statements(
        &b,
        &[
            "INSERT INTO w_upd VALUES ('restaged', 2)",
            "INSERT INTO w_upd VALUES ('staged', 2)",
        ],
    );
    let codes = after_holder_ends(&a, "COMMIT", task, "moved on and off").await;
    assert_eq!(
        codes.iter().map(Option::as_deref).collect::<Vec<_>>(),
        vec![Some("23505"), None]
    );
    assert_eq!(
        text_rows(&a, "SELECT a || '|' || v FROM w_upd ORDER BY 1").await,
        vec!["k|1", "moved|9", "restaged|1", "staged|2"],
    );

    exec(&a, "DROP TABLE w_upd").await;
    println!("✓ UPDATE onto and from a staged key, after the wait");
    Ok(())
}

/// Where the writer is released, each asserted: a `COMMIT` (the second writer answers
/// against the committed row), a `ROLLBACK`, a `ROLLBACK TO SAVEPOINT` that undoes a
/// write made after the savepoint (the writer is held on — the block still has staged
/// writes — and released at its `COMMIT`), and a client that goes away with its block
/// open. Each second writer's answer is 18.6's.
async fn test_writer_release_points() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;

    exec(&a, "DROP TABLE IF EXISTS w_rel").await;
    exec(&a, "CREATE TABLE w_rel (a TEXT PRIMARY KEY, v INT)").await;

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_rel VALUES ('c', 1)").await;
    exec(&a, "COMMIT").await;
    assert_eq!(
        sqlstate(&b, "INSERT INTO w_rel VALUES ('c', 2)")
            .await
            .as_deref(),
        Some("23505"),
        "after COMMIT: a committed row, and no wait"
    );

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_rel VALUES ('r', 1)").await;
    exec(&a, "ROLLBACK").await;
    assert_eq!(
        sqlstate(&b, "INSERT INTO w_rel VALUES ('r', 2)").await,
        None,
        "after ROLLBACK: free, and no wait"
    );

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_rel VALUES ('kept', 1)").await;
    exec(&a, "SAVEPOINT s1").await;
    exec(&a, "INSERT INTO w_rel VALUES ('undone', 1)").await;
    exec(&a, "ROLLBACK TO SAVEPOINT s1").await;
    let task = spawn_statements(
        &b,
        &[
            "INSERT INTO w_rel VALUES ('undone', 2)",
            "INSERT INTO w_rel VALUES ('kept', 2)",
        ],
    );
    let codes = after_holder_ends(&a, "COMMIT", task, "ROLLBACK TO, then COMMIT").await;
    assert_eq!(
        codes.iter().map(Option::as_deref).collect::<Vec<_>>(),
        vec![None, Some("23505")]
    );
    assert_eq!(
        text_rows(
            &a,
            "SELECT a || '|' || v FROM w_rel WHERE a IN ('kept','undone') ORDER BY 1"
        )
        .await,
        vec!["kept|1", "undone|2"],
    );

    // Disconnect: the server rolls the abandoned block back and releases the writer.
    let gone = connect().await;
    exec(&gone, "BEGIN").await;
    exec(&gone, "INSERT INTO w_rel VALUES ('dropped', 1)").await;
    let task = spawn_statements(&b, &["INSERT INTO w_rel VALUES ('dropped', 2)"]);
    settle().await;
    assert!(
        !task.is_finished(),
        "the writer is held while its connection is open"
    );
    drop(gone);
    let codes = task.await?;
    assert_eq!(codes, vec![None], "a disconnect releases the writer");

    exec(&a, "DROP TABLE w_rel").await;
    println!("✓ writer release points (commit / rollback / savepoint / disconnect)");
    Ok(())
}

/// `pg_advisory_xact_lock` **is the writer**: a block's call takes it and holds it to the
/// block's end, an autocommit call holds it for its statement, and a second connection's
/// call waits for it — then goes through when the holder commits or rolls back — or, under
/// a short `lock_timeout`, answers `55P03`. Each of those is 18.6's answer for one key
/// (measured over two connections). The last step pins what the mapping costs, so a
/// per-key lock turns it red on purpose: a *different* key waits too, where 18.6 grants it
/// at once (the repo's `EDGES.md` row 250).
async fn test_advisory_xact_lock_is_the_writer() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;
    let hurried = connect().await;
    exec(&hurried, "SET lock_timeout = '200ms'").await;
    let lock = "SELECT pg_advisory_xact_lock(7)";

    // A block's call holds it: a second call waits, then goes through at either end.
    for end in ["COMMIT", "ROLLBACK"] {
        exec(&a, "BEGIN").await;
        exec(&a, lock).await;
        // The holder takes it again at once.
        assert_eq!(sqlstate(&a, lock).await, None, "{end}: the holder again");
        let task = spawn_statements(&b, &[lock]);
        let codes = after_holder_ends(&a, end, task, end).await;
        assert_eq!(codes, vec![None], "{end}: the waiter goes through");
    }

    // Past a short bound the wait answers `55P03`; released at the `ROLLBACK`, it is free.
    exec(&a, "BEGIN").await;
    exec(&a, lock).await;
    assert_eq!(
        sqlstate(&hurried, lock).await.as_deref(),
        Some("55P03"),
        "a bounded wait on a held lock"
    );
    exec(&a, "ROLLBACK").await;
    assert_eq!(sqlstate(&hurried, lock).await, None, "released at ROLLBACK");

    // An autocommit call holds it for its statement only.
    exec(&a, lock).await;
    assert_eq!(
        sqlstate(&hurried, lock).await,
        None,
        "released at the autocommit statement's end"
    );

    // What the one writer costs: another key contends. 18.6 grants it at once.
    exec(&a, "BEGIN").await;
    exec(&a, lock).await;
    assert_eq!(
        sqlstate(&hurried, "SELECT pg_advisory_xact_lock(8)")
            .await
            .as_deref(),
        Some("55P03"),
        "every key is the one writer"
    );
    exec(&a, "ROLLBACK").await;

    println!(
        "✓ pg_advisory_xact_lock is the writer (waits, 55P03, released at the block's or statement's end)"
    );
    Ok(())
}

/// An **aborted** block holds the writer until its block ends — with no savepoint open
/// as with one. With none, 18.6 releases at the error and a second writer goes through at
/// once; pglet's waits for the `ROLLBACK`, with the same final rows. With a savepoint
/// open, 18.6 releases at the error what the aborted subtransaction wrote, so a second
/// writer of such a key goes through at once, and pglet's waits past the `ROLLBACK TO`
/// (the repo's `EDGES.md` row 230); the second half here writes only keys the block wrote
/// outside the aborted savepoint. Pinned so the release at the error,
/// when it lands, turns the first half red on purpose.
async fn test_aborted_block_holds_the_writer_until_it_ends()
-> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;

    exec(&a, "DROP TABLE IF EXISTS w_abort").await;
    exec(&a, "CREATE TABLE w_abort (a TEXT PRIMARY KEY, v INT)").await;

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_abort VALUES ('gone', 1)").await;
    exec(&a, "SELECT 1/0").await;
    let task = spawn_statements(&b, &["INSERT INTO w_abort VALUES ('gone', 2)"]);
    let codes = after_holder_ends(&a, "ROLLBACK", task, "aborted, no savepoint").await;
    assert_eq!(codes, vec![None]);

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_abort VALUES ('held', 1)").await;
    exec(&a, "SAVEPOINT s1").await;
    exec(&a, "SELECT 1/0").await;
    exec(&a, "ROLLBACK TO SAVEPOINT s1").await;
    exec(&a, "INSERT INTO w_abort VALUES ('after', 1)").await;
    let task = spawn_statements(
        &b,
        &[
            "INSERT INTO w_abort VALUES ('held', 2)",
            "INSERT INTO w_abort VALUES ('after', 2)",
        ],
    );
    let codes = after_holder_ends(&a, "COMMIT", task, "savepoint revived").await;
    assert_eq!(
        codes.iter().map(Option::as_deref).collect::<Vec<_>>(),
        vec![Some("23505"), Some("23505")]
    );
    assert_eq!(
        text_rows(&a, "SELECT a || '|' || v FROM w_abort ORDER BY 1").await,
        vec!["after|1", "gone|2", "held|1"],
    );

    exec(&a, "DROP TABLE w_abort").await;
    println!("✓ an aborted block holds the writer until it ends");
    Ok(())
}

/// The accept side: nothing that does not write waits. Two read-only blocks run side by
/// side, an autocommit read answers while another block holds the writer, and an
/// autocommit write answers at once beside a block that has only read — none
/// of them waits, as none does on 18.6.
async fn test_readers_never_wait() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect().await;
    exec(&a, "DROP TABLE IF EXISTS w_read").await;
    exec(&a, "CREATE TABLE w_read (id INT)").await;
    exec(&a, "INSERT INTO w_read VALUES (1)").await;

    let quick = |label: &'static str, started: std::time::Instant| {
        let took = started.elapsed();
        assert!(
            took < Duration::from_millis(250),
            "{label}: answered without waiting ({took:?})"
        );
    };

    // Two read-only blocks.
    exec(&a, "BEGIN").await;
    exec(&a, "SELECT count(*) FROM w_read").await;
    let started = std::time::Instant::now();
    exec(&b, "BEGIN").await;
    assert_eq!(count(&b, "w_read").await, 1);
    exec(&b, "COMMIT").await;
    quick("a second read-only block", started);

    // A write beside a block that has only read.
    let started = std::time::Instant::now();
    assert_eq!(sqlstate(&b, "INSERT INTO w_read VALUES (2)").await, None);
    quick("an autocommit write beside a read-only block", started);
    exec(&a, "ROLLBACK").await;
    assert_eq!(
        count(&a, "w_read").await,
        2,
        "the read-only block's ROLLBACK restored nothing over the write"
    );

    // An autocommit read beside a block holding the writer.
    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_read VALUES (3)").await;
    let started = std::time::Instant::now();
    assert_eq!(count(&b, "w_read").await, 2);
    quick("an autocommit read beside a writer", started);
    exec(&a, "ROLLBACK").await;

    exec(&a, "DROP TABLE w_read").await;
    println!("✓ readers never wait");
    Ok(())
}

/// A block decides its writes against what is committed when it **takes the writer**,
/// not what its `BEGIN` saw: a key another connection committed after the block's
/// `BEGIN` refuses the block's insert `23505`, and a parent another connection
/// deleted refuses the block's child `23503` — 18.6's answers. Both were silent
/// wrong commits while the block wrote from its `BEGIN` snapshot: two rows under a
/// primary key, a dangling child. The first `SELECT` is what makes the block's snapshot
/// older than the other connection's commit.
async fn test_a_block_writes_against_what_is_committed() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect().await;
    exec(&a, "DROP TABLE IF EXISTS w_ch").await;
    exec(&a, "DROP TABLE IF EXISTS w_pa").await;
    exec(&a, "DROP TABLE IF EXISTS w_k").await;
    exec(&a, "CREATE TABLE w_k (k INT PRIMARY KEY)").await;
    exec(&a, "CREATE TABLE w_pa (id INT PRIMARY KEY)").await;
    exec(
        &a,
        "CREATE TABLE w_ch (id INT, pa INT REFERENCES w_pa (id))",
    )
    .await;
    exec(&a, "INSERT INTO w_pa VALUES (1)").await;

    exec(&a, "BEGIN").await;
    assert_eq!(count(&a, "w_k").await, 0);
    assert_eq!(sqlstate(&b, "INSERT INTO w_k VALUES (1)").await, None);
    assert_eq!(
        sqlstate(&a, "INSERT INTO w_k VALUES (1)").await.as_deref(),
        Some("23505"),
        "a key committed after BEGIN refuses the block's insert"
    );
    exec(&a, "ROLLBACK").await;
    assert_eq!(count(&a, "w_k").await, 1, "one k, under the primary key");

    exec(&a, "BEGIN").await;
    assert_eq!(count(&a, "w_pa").await, 1);
    assert_eq!(sqlstate(&b, "DELETE FROM w_pa WHERE id = 1").await, None);
    assert_eq!(
        sqlstate(&a, "INSERT INTO w_ch VALUES (1, 1)")
            .await
            .as_deref(),
        Some("23503"),
        "a parent deleted after BEGIN refuses the block's child"
    );
    exec(&a, "ROLLBACK").await;
    assert_eq!(count(&a, "w_ch").await, 0, "no dangling child");

    for table in ["w_ch", "w_pa", "w_k"] {
        exec(&a, &format!("DROP TABLE {table}")).await;
    }
    println!("✓ a block writes against what is committed when it takes the writer");
    Ok(())
}

/// The doors that take the writer beside DML: a `SELECT … FOR UPDATE` in a block (F1 —
/// the second writer's `UPDATE` waits and the final value carries both increments, where
/// the lock-free design lost one), a standalone `nextval` (N1), and a `COPY …
/// FROM STDIN` (which drew a sequence without waiting before it took the writer). Each
/// second statement is seen waiting, then answers after the holder's `COMMIT`.
async fn test_every_writing_door_waits() -> Result<(), Box<dyn std::error::Error>> {
    use futures_util::SinkExt;

    let a = connect().await;
    let b = connect_patient().await;
    exec(&a, "DROP TABLE IF EXISTS w_fu").await;
    exec(&a, "DROP TABLE IF EXISTS w_copy").await;
    exec(&a, "DROP SEQUENCE IF EXISTS w_seq").await;
    exec(&a, "CREATE TABLE w_fu (id INT PRIMARY KEY, v INT)").await;
    exec(&a, "INSERT INTO w_fu VALUES (1, 10)").await;
    exec(&a, "CREATE SEQUENCE w_seq").await;
    exec(&a, "CREATE TABLE w_copy (id SERIAL, t TEXT)").await;

    // FOR UPDATE, then an UPDATE of the row it read.
    exec(&a, "BEGIN").await;
    let v: i32 = query_one_i32(&a, "SELECT v FROM w_fu WHERE id = 1 FOR UPDATE").await;
    let task = spawn_statements(&b, &["UPDATE w_fu SET v = v + 1 WHERE id = 1"]);
    // The block has only read, with `FOR UPDATE`: that read is what B waits on. Without
    // the settle B's statement can reach the server after A's own `UPDATE` below, which
    // takes the writer by itself, and the case would not discriminate.
    settle().await;
    assert!(
        !task.is_finished(),
        "an UPDATE waits for a block whose only statement is a FOR UPDATE read"
    );
    exec(&a, &format!("UPDATE w_fu SET v = {} WHERE id = 1", v + 1)).await;
    let codes = after_holder_ends(&a, "COMMIT", task, "FOR UPDATE").await;
    assert_eq!(codes, vec![None]);
    assert_eq!(
        query_one_i32(&a, "SELECT v FROM w_fu WHERE id = 1").await,
        12,
        "F1: both increments"
    );

    // A standalone nextval waits for a block holding the writer.
    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_fu VALUES (2, 20)").await;
    let task = spawn_statements(&b, &["SELECT nextval('w_seq')"]);
    let codes = after_holder_ends(&a, "COMMIT", task, "nextval").await;
    assert_eq!(codes, vec![None]);

    // COPY FROM waits, and its column default draws only once it holds the writer.
    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_fu VALUES (3, 30)").await;
    let copier = std::sync::Arc::clone(&b);
    let task = tokio::spawn(async move {
        let sink = copier
            .copy_in("COPY w_copy (t) FROM STDIN")
            .await
            .expect("COPY starts");
        futures_util::pin_mut!(sink);
        sink.send(bytes::Bytes::from_static(b"one\ntwo\n"))
            .await
            .expect("COPY data");
        sink.finish().await.map(|_| ()).map_err(|e| e.to_string())
    });
    settle().await;
    assert!(
        !task.is_finished(),
        "COPY FROM waits for the holder's block"
    );
    exec(&a, "COMMIT").await;
    task.await?.map_err(|e| format!("COPY: {e}"))?;
    assert_eq!(count(&a, "w_copy").await, 2);

    exec(&a, "DROP TABLE w_fu").await;
    exec(&a, "DROP TABLE w_copy").await;
    exec(&a, "DROP SEQUENCE w_seq").await;
    println!("✓ FOR UPDATE, nextval and COPY FROM wait for the writer");
    Ok(())
}

/// Row 227's shapes: a block whose `UPDATE` meets a key another connection moved away and
/// inserted again after the block's `BEGIN` (x1), and a block whose `UPDATE` precedes the
/// other connection's move (x2). The block's write decides against what is committed when
/// it takes the writer, and its `COMMIT` derives each row's old index entry from the same
/// state — so the index agrees with the heap: `ORDER BY` the key reads each row once, a
/// point lookup on a moved key and on the re-inserted one finds the heap's row, a key a row
/// held and gave up is insertable, and a held key refuses `23505`. Before the writer, x1
/// committed two rows under `id = 1` and the key-ordered scan read four; red with the
/// re-base at acquisition removed.
async fn test_the_index_agrees_with_the_heap_after_a_moved_key()
-> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;
    let setup = |t: &str| {
        format!(
            "DROP TABLE IF EXISTS {t}; CREATE TABLE {t} (id INT PRIMARY KEY, c INT); INSERT INTO {t} VALUES (1, 0), (2, 0)"
        )
    };
    // x1: the block has only read when the other connection moves the key.
    exec(&a, &setup("w_x1")).await;
    exec(&a, "BEGIN").await;
    assert_eq!(count(&a, "w_x1").await, 2);
    assert_eq!(
        sqlstate(&b, "UPDATE w_x1 SET id = 3 WHERE id = 1").await,
        None
    );
    assert_eq!(sqlstate(&b, "INSERT INTO w_x1 VALUES (1, 7)").await, None);
    assert_eq!(
        text_rows(
            &a,
            "UPDATE w_x1 SET c = 5 WHERE id = 1 RETURNING id || '|' || c"
        )
        .await,
        vec!["1|5"],
        "x1: the block updates the row now under the key"
    );
    exec(&a, "COMMIT").await;
    // x2: the block writes first, and the move waits for it.
    exec(&a, &setup("w_x2")).await;
    exec(&a, "BEGIN").await;
    exec(&a, "UPDATE w_x2 SET c = 5 WHERE id = 1").await;
    let task = spawn_statements(
        &b,
        &[
            "UPDATE w_x2 SET id = 3 WHERE id = 1",
            "INSERT INTO w_x2 VALUES (1, 7)",
        ],
    );
    let codes = after_holder_ends(&a, "COMMIT", task, "x2").await;
    assert_eq!(
        codes,
        vec![None, None],
        "x2: the move and the insert after the wait"
    );
    for (t, rows) in [
        ("w_x1", ["1|5", "2|0", "3|0"]),
        ("w_x2", ["1|7", "2|0", "3|5"]),
    ] {
        assert_eq!(
            text_rows(&a, &format!("SELECT id || '|' || c FROM {t} ORDER BY id")).await,
            rows,
            "{t}: the key-ordered scan"
        );
        assert_eq!(
            text_rows(&a, &format!("SELECT id || '|' || c FROM {t}"))
                .await
                .len(),
            3,
            "{t}: the heap"
        );
        for (key, row) in [(1, rows[0]), (3, rows[2])] {
            assert_eq!(
                text_rows(
                    &a,
                    &format!("SELECT id || '|' || c FROM {t} WHERE id = {key}")
                )
                .await,
                vec![row],
                "{t}: the point lookup on {key}"
            );
            assert_eq!(
                sqlstate(&a, &format!("INSERT INTO {t} VALUES ({key}, 0)"))
                    .await
                    .as_deref(),
                Some("23505"),
                "{t}: {key} is held"
            );
        }
        assert_eq!(
            sqlstate(&a, &format!("DELETE FROM {t} WHERE id = 3")).await,
            None
        );
        assert_eq!(
            sqlstate(&a, &format!("INSERT INTO {t} VALUES (3, 9)")).await,
            None,
            "{t}: a freed key is insertable"
        );
        exec(&a, &format!("DROP TABLE {t}")).await;
    }
    println!("✓ the index agrees with the heap after a moved and re-inserted key (row 227)");
    Ok(())
}

/// A plan failing because of another connection's uncommitted catalog change is not the
/// statement's answer: an `UPDATE` of a column another connection's open block dropped
/// waits for that block and
/// is planned again against what it left — it succeeds after the block's `ROLLBACK` and
/// answers `42703` after its `COMMIT`, 18.6's answers (its parse analysis waits on the
/// `ALTER`'s lock). The extended protocol's `Parse` waits the same way. Red with the plan's
/// failure answered before the wait (`42703` at once, over a column the rollback restores).
async fn test_a_plan_waits_out_a_holders_uncommitted_catalog()
-> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;
    exec(&a, "DROP TABLE IF EXISTS w_dc").await;
    exec(&a, "CREATE TABLE w_dc (id INT, c INT)").await;
    exec(&a, "INSERT INTO w_dc VALUES (1, 1)").await;
    for (end, answer) in [("ROLLBACK", None), ("COMMIT", Some("42703"))] {
        exec(&a, "BEGIN").await;
        exec(&a, "ALTER TABLE w_dc DROP COLUMN c").await;
        let task = spawn_statements(&b, &["UPDATE w_dc SET c = 9"]);
        let codes = after_holder_ends(&a, end, task, end).await;
        assert_eq!(
            codes[0].as_deref(),
            answer,
            "simple protocol, after the {end}"
        );
    }
    exec(&a, "ALTER TABLE w_dc ADD COLUMN c INT").await;
    exec(&a, "BEGIN").await;
    exec(&a, "ALTER TABLE w_dc DROP COLUMN c").await;
    let prepared = std::sync::Arc::clone(&b);
    let task = tokio::spawn(async move {
        prepared
            .execute("UPDATE w_dc SET c = $1", &[&9_i32])
            .await
            .map_err(|e| e.code().map(|c| c.code().to_string()))
    });
    settle().await;
    assert!(
        !task.is_finished(),
        "the extended Parse waits for the block"
    );
    exec(&a, "ROLLBACK").await;
    assert_eq!(task.await?, Ok(1), "extended protocol, after the ROLLBACK");
    assert_eq!(text_rows(&a, "SELECT c::text FROM w_dc").await, vec!["9"]);
    exec(&a, "DROP TABLE w_dc").await;
    println!("✓ a plan failing against a holder's uncommitted catalog waits and plans again");
    Ok(())
}

/// A `COPY`'s column list, and a set operation's column counts, are resolved against
/// the catalog as a plan is — so neither answers from another connection's uncommitted
/// catalog. Behind a block that dropped `c`: `COPY t (c) FROM STDIN` and `COPY t (c) TO
/// STDOUT` (whose plan does not resolve the list) wait, and after the block's `ROLLBACK`
/// the first ingests and the second reads; `SELECT * FROM t UNION ALL SELECT 7, 8`
/// (whose `42601` is pglet's analysis, over the table's width) waits and reads, in both
/// protocols. 18.6's `COPY` opens the table under a lock the `ALTER` holds before it
/// resolves the list, and its parse analysis of the `SELECT *` does the same. Red with
/// the list resolved outside the plan wait (`42703` at once for each `COPY`), and with
/// every `42601` taken as the statement's own (the set operation's `42601` at once).
async fn test_a_copy_list_and_a_set_op_wait_out_a_holders_uncommitted_catalog()
-> Result<(), Box<dyn std::error::Error>> {
    use futures_util::{SinkExt, TryStreamExt};

    let a = connect().await;
    let b = connect_patient().await;
    exec(&a, "DROP TABLE IF EXISTS w_cl").await;
    exec(&a, "CREATE TABLE w_cl (a INT, c INT)").await;
    exec(&a, "INSERT INTO w_cl VALUES (1, 7)").await;

    exec(&a, "BEGIN").await;
    exec(&a, "ALTER TABLE w_cl DROP COLUMN c").await;
    let copier = std::sync::Arc::clone(&b);
    let task = tokio::spawn(async move {
        let sink = copier
            .copy_in("COPY w_cl (c) FROM STDIN")
            .await
            .map_err(|e| e.code().map(|c| c.code().to_string()))?;
        futures_util::pin_mut!(sink);
        sink.send(bytes::Bytes::from_static(b"9\n"))
            .await
            .map_err(|e| e.code().map(|c| c.code().to_string()))?;
        sink.finish()
            .await
            .map_err(|e| e.code().map(|c| c.code().to_string()))
    });
    settle().await;
    assert!(!task.is_finished(), "COPY (c) FROM waits for the block");
    exec(&a, "ROLLBACK").await;
    assert_eq!(task.await?, Ok(1), "COPY (c) FROM, after the ROLLBACK");

    exec(&a, "BEGIN").await;
    exec(&a, "ALTER TABLE w_cl DROP COLUMN c").await;
    let reader = std::sync::Arc::clone(&b);
    let task = tokio::spawn(async move {
        let stream = reader
            .copy_out("COPY w_cl (c) TO STDOUT")
            .await
            .map_err(|e| e.code().map(|c| c.code().to_string()))?;
        let chunks: Vec<bytes::Bytes> = stream
            .try_collect()
            .await
            .map_err(|e| e.code().map(|c| c.code().to_string()))?;
        Ok::<_, Option<String>>(chunks.concat())
    });
    settle().await;
    assert!(!task.is_finished(), "COPY (c) TO waits for the block");
    exec(&a, "ROLLBACK").await;
    assert_eq!(
        task.await?,
        Ok(b"7\n9\n".to_vec()),
        "COPY (c) TO, after the ROLLBACK"
    );

    let set_op = "SELECT * FROM w_cl UNION ALL SELECT 7, 8";
    exec(&a, "BEGIN").await;
    exec(&a, "ALTER TABLE w_cl DROP COLUMN c").await;
    let task = spawn_statements(&b, &[set_op]);
    let codes = after_holder_ends(&a, "ROLLBACK", task, "set operation").await;
    assert_eq!(codes, vec![None], "simple protocol, after the ROLLBACK");
    exec(&a, "BEGIN").await;
    exec(&a, "ALTER TABLE w_cl ADD COLUMN e INT").await;
    let prepared = std::sync::Arc::clone(&b);
    let task = tokio::spawn(async move {
        prepared
            .query(set_op, &[])
            .await
            .map(|rows| rows.len())
            .map_err(|e| e.code().map(|c| c.code().to_string()))
    });
    settle().await;
    assert!(
        !task.is_finished(),
        "the extended Parse of the set operation waits for the block"
    );
    exec(&a, "ROLLBACK").await;
    assert_eq!(task.await?, Ok(3), "extended protocol, after the ROLLBACK");
    exec(&a, "DROP TABLE w_cl").await;
    println!("✓ a COPY column list and a set operation wait out a holder's uncommitted catalog");
    Ok(())
}

/// The other polarity: a plan that fails for its own reason answers at once, whatever
/// another connection's block has done to the catalog — behind a block that created a
/// table these statements never name, and behind one that did so and then went aborted, a
/// column no catalog has, a cast no catalog decides and a function no catalog has answer
/// what they answer with no block open, in both protocols, with no wait. 18.6 answers
/// them at once: its parse analysis takes no lock the block holds. Red with every
/// non-syntax plan failure behind a changed catalog taken as the block's (each waits out
/// the block, here past the settle).
async fn test_a_plans_own_failure_answers_at_once() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect_patient().await;
    exec(&a, "DROP TABLE IF EXISTS w_own").await;
    exec(&a, "CREATE TABLE w_own (i INT)").await;
    exec(&a, "INSERT INTO w_own VALUES (1)").await;
    let statements = [
        "SELECT nosuchcol FROM w_own",
        "SELECT 'abc'::int",
        "SELECT no_such_fn(1)",
        "UPDATE w_own SET nosuchcol = 1",
    ];
    let mut own = Vec::new();
    for sql in statements {
        own.push(sqlstate(&b, sql).await);
    }
    assert!(own.iter().all(Option::is_some), "{own:?}");
    let parse_own = b
        .prepare("SELECT nosuchcol FROM w_own")
        .await
        .err()
        .and_then(|e| e.code().map(|c| c.code().to_string()));
    for (label, holder) in [
        (
            "a table created",
            &["BEGIN", "CREATE TABLE w_own_new (x INT)"][..],
        ),
        (
            "a table created, then aborted",
            &["BEGIN", "CREATE TABLE w_own_new (x INT)", "SELECT 1/0"][..],
        ),
    ] {
        for sql in holder {
            exec(&a, sql).await;
        }
        let task = spawn_statements(&b, &statements);
        settle().await;
        assert!(task.is_finished(), "{label}: answered at once");
        assert_eq!(task.await?, own, "{label}");
        let prepared = std::sync::Arc::clone(&b);
        let parse = tokio::spawn(async move {
            prepared
                .prepare("SELECT nosuchcol FROM w_own")
                .await
                .err()
                .and_then(|e| e.code().map(|c| c.code().to_string()))
        });
        settle().await;
        assert!(
            parse.is_finished(),
            "{label}: the extended Parse answers at once"
        );
        assert_eq!(parse.await?, parse_own, "{label}: extended protocol");
        exec(&a, "ROLLBACK").await;
    }
    exec(&a, "DROP TABLE w_own").await;
    println!("✓ a plan failing for its own reason answers at once behind a catalog change");
    Ok(())
}

/// A `REPEATABLE READ` / `SERIALIZABLE` block's snapshot is taken by its first statement,
/// not by `BEGIN` (18.6, `mvcc.sgml`: "as of the start of the first non-transaction-control
/// statement"): a block whose first statement is its write reads what another connection
/// committed between the two and is not refused — `UPDATE … c * 10` over a committed `5`
/// leaves `50`, 18.6's answer. Red with the check measured from `BEGIN` (`40001`, `5`).
async fn test_a_first_statement_write_takes_the_snapshot() -> Result<(), Box<dyn std::error::Error>>
{
    let a = connect().await;
    let b = connect().await;
    exec(&a, "DROP TABLE IF EXISTS w_fs").await;
    exec(&a, "CREATE TABLE w_fs (id INT PRIMARY KEY, c INT)").await;
    for begin in [
        "BEGIN ISOLATION LEVEL REPEATABLE READ",
        "BEGIN ISOLATION LEVEL SERIALIZABLE",
    ] {
        exec(&a, "DELETE FROM w_fs").await;
        exec(&a, "INSERT INTO w_fs VALUES (1, 0)").await;
        exec(&a, begin).await;
        assert_eq!(sqlstate(&b, "UPDATE w_fs SET c = 5").await, None, "{begin}");
        assert_eq!(
            sqlstate(&a, "UPDATE w_fs SET c = c * 10").await,
            None,
            "{begin}"
        );
        assert_eq!(sqlstate(&a, "COMMIT").await, None, "{begin}");
        assert_eq!(query_one_i32(&b, "SELECT c FROM w_fs").await, 50, "{begin}");
    }
    exec(&a, "DROP TABLE w_fs").await;
    println!("✓ a block's first-statement write takes its snapshot (no 40001)");
    Ok(())
}

/// A `REPEATABLE READ` or `SERIALIZABLE` block keeps its snapshot and is refused `40001`
/// at its first write when another connection committed since it began — S13RR / S13SER:
/// the second read `0`, the `UPDATE` `40001`, the final value `5`, 18.6's answers where the
/// block had committed `50` over the other connection's write. With nothing committed
/// since, it writes; `READ COMMITTED` re-bases and writes over the commit (`55`). Red with
/// the check removed from `TransactionSession::take_writer`.
async fn test_repeatable_read_refuses_a_write_past_a_commit()
-> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect().await;
    exec(&a, "DROP TABLE IF EXISTS w_rr").await;
    exec(&a, "CREATE TABLE w_rr (id INT PRIMARY KEY, c INT)").await;
    for (begin, committed, answer, last) in [
        (
            "BEGIN ISOLATION LEVEL REPEATABLE READ",
            true,
            Some("40001"),
            5,
        ),
        ("BEGIN ISOLATION LEVEL SERIALIZABLE", true, Some("40001"), 5),
        ("BEGIN ISOLATION LEVEL REPEATABLE READ", false, None, 50),
        ("BEGIN", true, None, 55),
    ] {
        let cell = format!("{begin}, a commit between {committed}");
        exec(&a, "DELETE FROM w_rr").await;
        exec(&a, "INSERT INTO w_rr VALUES (1, 0)").await;
        exec(&a, begin).await;
        assert_eq!(query_one_i32(&a, "SELECT c FROM w_rr").await, 0, "{cell}");
        if committed {
            assert_eq!(sqlstate(&b, "UPDATE w_rr SET c = 5").await, None, "{cell}");
        }
        if answer.is_some() {
            assert_eq!(
                query_one_i32(&a, "SELECT c FROM w_rr").await,
                0,
                "{cell}: the snapshot's second read"
            );
        }
        assert_eq!(
            sqlstate(&a, "UPDATE w_rr SET c = c + 50").await.as_deref(),
            answer,
            "{cell}"
        );
        exec(
            &a,
            if answer.is_some() {
                "ROLLBACK"
            } else {
                "COMMIT"
            },
        )
        .await;
        assert_eq!(
            query_one_i32(&b, "SELECT c FROM w_rr").await,
            last,
            "{cell}"
        );
    }
    exec(&a, "DROP TABLE w_rr").await;
    println!("✓ REPEATABLE READ / SERIALIZABLE refuse a write past a commit (40001)");
    Ok(())
}

/// `lock_timeout` bounds the wait: `SHOW lock_timeout` answers pglet's default `1s`, a
/// write that waits past the session's bound answers `55P03` (`canceling statement due
/// to lock timeout`) — in a block it aborts the block (`25P02` until `ROLLBACK`) — and
/// `SET lock_timeout = 0` waits without a bound (still waiting past two seconds).
async fn test_lock_timeout_bounds_the_wait() -> Result<(), Box<dyn std::error::Error>> {
    let a = connect().await;
    let b = connect().await;
    exec(&a, "DROP TABLE IF EXISTS w_to").await;
    exec(&a, "CREATE TABLE w_to (id INT)").await;
    assert_eq!(text_rows(&b, "SHOW lock_timeout").await, vec!["1s"]);

    exec(&a, "BEGIN").await;
    exec(&a, "INSERT INTO w_to VALUES (1)").await;

    let started = std::time::Instant::now();
    assert_eq!(
        sqlstate(&b, "INSERT INTO w_to VALUES (2)").await.as_deref(),
        Some("55P03"),
        "the default bound"
    );
    let took = started.elapsed();
    assert!(
        took >= Duration::from_millis(900) && took < Duration::from_secs(5),
        "about the 1s default: {took:?}"
    );

    exec(&b, "SET lock_timeout = '200ms'").await;
    assert_eq!(text_rows(&b, "SHOW lock_timeout").await, vec!["200ms"]);
    exec(&b, "BEGIN").await;
    assert_eq!(
        sqlstate(&b, "INSERT INTO w_to VALUES (3)").await.as_deref(),
        Some("55P03")
    );
    assert_eq!(
        sqlstate(&b, "SELECT 1").await.as_deref(),
        Some("25P02"),
        "the timeout aborted b's block"
    );
    exec(&b, "ROLLBACK").await;

    exec(&b, "SET lock_timeout = 0").await;
    assert_eq!(text_rows(&b, "SHOW lock_timeout").await, vec!["0"]);
    let b = std::sync::Arc::new(b);
    let task = spawn_statements(&b, &["INSERT INTO w_to VALUES (4)"]);
    tokio::time::sleep(Duration::from_millis(2200)).await;
    assert!(
        !task.is_finished(),
        "lock_timeout = 0 waits past two seconds"
    );
    exec(&a, "COMMIT").await;
    assert_eq!(task.await?, vec![None]);
    assert_eq!(count(&a, "w_to").await, 2);

    exec(&a, "DROP TABLE w_to").await;
    println!("✓ lock_timeout bounds the wait (55P03), and 0 waits without a bound");
    Ok(())
}

#[tokio::main]
async fn main() {
    eprintln!("[test] Building pglet_server...");
    if !build_server() {
        eprintln!("[test] Build failed!");
        std::process::exit(1);
    }

    let port = find_open_port(PGLET_TEST_PORT..DYNAMIC_PORT_END);
    TEST_PORT.set(port).expect("TEST_PORT already set");
    eprintln!("[test] Spawning pglet_server on port {port}...");
    // In-memory storage (no `--data-dir`): the MVCC tests don't need persistence.
    let server = spawn_server(port, None);

    if !wait_for_ready(port).await {
        eprintln!("[test] Server failed to start!");
        hard_kill(server);
        std::process::exit(1);
    }
    eprintln!("[test] Server ready\n");

    println!("=== Concurrent Client Test Suite ===\n");

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

    println!("--- Transaction Isolation ---");
    run_test!("dirty read prevention", test_dirty_read_prevention());
    run_test!("snapshot isolation", test_snapshot_isolation());
    run_test!("read your own writes", test_read_your_writes());
    run_test!("rollback visibility", test_rollback_visibility());
    run_test!(
        "aborted block commits nothing",
        test_aborted_block_commits_nothing()
    );
    run_test!(
        "comment-prefixed exit unwedges the session",
        test_comment_prefixed_exit_unwedges_the_session()
    );

    println!("\n--- Concurrent Access ---");
    run_test!("concurrent inserts", test_concurrent_inserts());
    run_test!(
        "multiple readers during write",
        test_multiple_readers_during_write()
    );

    println!("\n--- Stress Tests ---");
    run_test!("stress (20 connections)", test_stress_many_connections());
    run_test!("concurrent DDL", test_concurrent_ddl());
    run_test!(
        "concurrent DDL vs rollback (no clobber)",
        test_concurrent_ddl_rollback_no_clobber()
    );
    run_test!(
        "concurrent table vs index DDL rollback (no divergence)",
        test_concurrent_table_vs_index_ddl_rollback_no_divergence()
    );
    run_test!(
        "DROP-in-txn rollback restores committed rows",
        test_drop_in_txn_rollback_restores_committed_rows()
    );
    run_test!(
        "CREATE INDEX in txn rollback (no phantom)",
        test_create_index_in_txn_rollback_no_phantom()
    );
    run_test!(
        "DO-block DDL rolls back with its transaction",
        test_do_block_ddl_rolls_back()
    );
    run_test!(
        "DO-block DDL persists on COMMIT",
        test_do_block_ddl_commits()
    );
    run_test!("write conflict", test_write_conflict());

    println!("\n--- The writer ---");
    run_test!(
        "a second writer waits for the holder",
        test_second_writer_waits_for_the_holder()
    );
    run_test!(
        "committed-row and own-key conflicts unchanged",
        test_committed_and_own_key_conflicts_unchanged()
    );
    run_test!(
        "ON CONFLICT actions after the wait",
        test_on_conflict_actions_after_the_wait()
    );
    run_test!(
        "DO NOTHING beside a staged key after the wait",
        test_do_nothing_beside_a_staged_key_after_the_wait()
    );
    run_test!(
        "index shapes after the wait",
        test_index_shapes_after_the_wait()
    );
    run_test!(
        "column DDL then a second writer",
        test_column_ddl_then_a_second_writer()
    );
    run_test!(
        "UPDATE onto a staged key after the wait",
        test_update_onto_a_staged_key_after_the_wait()
    );
    run_test!("writer release points", test_writer_release_points());
    run_test!(
        "an aborted block holds the writer until it ends",
        test_aborted_block_holds_the_writer_until_it_ends()
    );
    run_test!(
        "pg_advisory_xact_lock is the writer",
        test_advisory_xact_lock_is_the_writer()
    );
    run_test!("readers never wait", test_readers_never_wait());
    run_test!(
        "a block writes against what is committed",
        test_a_block_writes_against_what_is_committed()
    );
    run_test!("every writing door waits", test_every_writing_door_waits());
    run_test!(
        "lock_timeout bounds the wait",
        test_lock_timeout_bounds_the_wait()
    );
    run_test!(
        "the index agrees with the heap after a moved key",
        test_the_index_agrees_with_the_heap_after_a_moved_key()
    );
    run_test!(
        "a plan waits out a holder's uncommitted catalog",
        test_a_plan_waits_out_a_holders_uncommitted_catalog()
    );
    run_test!(
        "REPEATABLE READ refuses a write past a commit",
        test_repeatable_read_refuses_a_write_past_a_commit()
    );
    run_test!(
        "a plan's own failure answers at once",
        test_a_plans_own_failure_answers_at_once()
    );
    run_test!(
        "a first-statement write takes the snapshot",
        test_a_first_statement_write_takes_the_snapshot()
    );
    run_test!(
        "a COPY list and a set operation wait out a holder's catalog",
        test_a_copy_list_and_a_set_op_wait_out_a_holders_uncommitted_catalog()
    );

    println!("\n--- Connection Lifecycle ---");
    run_test!("connection drop rollback", test_connection_drop_rollback());

    println!("\n=== Results ===");
    println!("Passed: {passed}");
    println!("Failed: {failed}");

    eprintln!("\n[test] Shutting down server...");
    hard_kill(server);

    if failed > 0 {
        std::process::exit(1);
    }
}
