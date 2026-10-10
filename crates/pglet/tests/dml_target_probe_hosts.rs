//! A simple `UPDATE` / `DELETE`'s index probe on the embed's two backends.
//!
//! Two properties the `pglet_sql` cost guard (over its scan-and-filter oracle, with no
//! transaction) cannot see:
//!
//! - **An interval that names no key answers no row on both backends.** The memory
//!   backend's range lookup handed such an interval to `BTreeMap::range`, which panics, so
//!   `SELECT … WHERE k BETWEEN 4 AND 2` over an indexed key dropped the connection (and,
//!   inside a block, left the block's earlier writes standing).
//! - **A probed write reads no whole table on the B+tree, whose writes all run inside a
//!   storage transaction.** There a range lookup is itself a read of the whole table
//!   (`index_range_lookup_in_tx`), so a probe saves the scan only where it is an equality —
//!   which is why a write takes no other kind. Counted through the B+tree wrapper's
//!   `table_scans_in_tx`, which every in-transaction table read moves.

#![allow(clippy::unwrap_used, clippy::panic)]

use pglet::{Database, StorageAdapter, StorageBackend};
use pglet_sql::executor::{EvalEnv, SequenceSession, execute_with_storage};
use pglet_sql::planner::Plan;
use pglet_sql::types::Value;

/// `sql`'s last result: its command tag and its rows, or `ERR <SQLSTATE>`.
fn answer(db: &mut Database, sql: &str) -> String {
    match db.execute(sql) {
        Ok(results) => results
            .last()
            .map_or_else(String::new, |r| format!("{} {:?}", r.command_tag, r.rows)),
        Err(e) => format!("ERR {}", e.sqlstate()),
    }
}

fn keyed(db: &mut Database) {
    db.execute("CREATE TABLE z (k INT PRIMARY KEY, v TEXT)")
        .unwrap();
    db.execute("INSERT INTO z VALUES (1, 'a'), (2, 'b'), (3, 'c'), (4, 'd')")
        .unwrap();
}

#[test]
fn an_interval_naming_no_key_answers_no_row_on_both_backends() {
    for (backend, mut db) in [
        ("memory", Database::in_memory()),
        ("btree", Database::with_btree()),
    ] {
        keyed(&mut db);
        for (sql, want) in [
            ("SELECT k FROM z WHERE k BETWEEN 4 AND 2", "SELECT 0 []"),
            ("SELECT k FROM z WHERE k > 3 AND k < 3", "SELECT 0 []"),
            ("SELECT k FROM z WHERE k >= 3 AND k < 3", "SELECT 0 []"),
            ("SELECT k FROM z WHERE k > 3 AND k <= 3", "SELECT 0 []"),
            (
                "SELECT k FROM z WHERE k >= 3 AND k <= 3",
                "SELECT 1 [[Int4(3)]]",
            ),
            ("DELETE FROM z WHERE k > 3 AND k < 3", "DELETE 0 []"),
            (
                "UPDATE z SET v = 'x' WHERE k BETWEEN 4 AND 2",
                "UPDATE 0 []",
            ),
        ] {
            assert_eq!(answer(&mut db, sql), want, "{backend}: {sql}");
        }
        // Bound, through the embed's parameter door.
        let bound = db
            .execute_bound("DELETE FROM z WHERE k BETWEEN $1 AND $2", 0, |_| {
                Ok::<_, ()>(vec![Some(Value::Int4(3)), Some(Value::Int4(1))])
            })
            .unwrap_or_else(|e| panic!("{backend}: {e:?}"));
        assert_eq!(bound.command_tag, "DELETE 0", "{backend}: bound");
        // Inside a block: the statement answers, and the block's earlier write is the
        // block's to keep or drop.
        for sql in [
            "BEGIN",
            "INSERT INTO z VALUES (50, 'n')",
            "UPDATE z SET v = 'x' WHERE k BETWEEN 4 AND 2",
            "SELECT k FROM z WHERE k > 3 AND k < 3",
            "ROLLBACK",
        ] {
            let got = answer(&mut db, sql);
            assert!(!got.starts_with("ERR"), "{backend}: {sql}: {got}");
        }
        assert_eq!(
            answer(&mut db, "SELECT count(*) FROM z WHERE k = 50"),
            "SELECT 1 [[Int8(0)]]",
            "{backend}: the rolled-back insert"
        );
    }
}

/// The connection the B+tree transaction below is opened for.
const CONN: i32 = 7;

/// Whether `plan` is a write carrying an index probe.
fn carries_probe(plan: &Plan) -> bool {
    match plan {
        Plan::Update { target_probe, .. } | Plan::Delete { target_probe, .. } => {
            target_probe.is_some()
        }
        other => panic!("expected a write, got {other:?}"),
    }
}

#[test]
fn a_probed_write_reads_no_whole_table_inside_a_btree_transaction() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT PRIMARY KEY, k INT, v TEXT)")
        .unwrap();
    db.execute("CREATE INDEX t_k ON t (k)").unwrap();
    db.execute("INSERT INTO t VALUES (1, 1, 'a'), (2, 2, 'b'), (3, 2, 'c'), (4, 4, 'd')")
        .unwrap();
    let (mut schema, mut storage) = db.into_parts();
    let mut session = SequenceSession::new();
    storage.begin(CONN).unwrap();
    // `(statement, whether the default build probes it)`. The stripped build probes none.
    let cases = [
        ("UPDATE t SET v = 'x' WHERE id = 2", true),
        ("UPDATE t SET v = 'y' WHERE k = 2 AND v IS NOT NULL", true),
        ("DELETE FROM t WHERE id = 4", true),
        ("UPDATE t SET v = 'z' WHERE id > 1", false),
        ("DELETE FROM t WHERE k BETWEEN 3 AND 9", false),
        ("UPDATE t SET v = 'w' WHERE v = 'a'", false),
    ];
    for (sql, probed) in cases {
        let plan = pglet_sql::parser::parse_single(sql, &schema)
            .unwrap()
            .expect("a statement");
        assert_eq!(
            carries_probe(&plan),
            probed && cfg!(feature = "index_acceleration"),
            "{sql}: probe"
        );
        let scans = |storage: &StorageBackend| {
            let StorageBackend::Btree(btree) = storage else {
                panic!("expected the B+tree backend");
            };
            btree.table_scans_in_tx()
        };
        let before = scans(&storage);
        {
            let mut adapter = StorageAdapter::new(&mut storage, CONN);
            execute_with_storage(
                &plan,
                &mut schema,
                &mut adapter,
                EvalEnv::EMPTY,
                &mut session,
            )
            .unwrap_or_else(|e| panic!("{sql}: {e}"));
        }
        let read = scans(&storage) - before;
        // The property: a write that carries a probe reads no whole table. A write that
        // carries none reads one, its scan.
        let expected = u64::from(!carries_probe(&plan));
        assert_eq!(
            read, expected,
            "{sql}: whole-table reads in the transaction"
        );
    }
    storage.rollback(CONN).unwrap();
}
