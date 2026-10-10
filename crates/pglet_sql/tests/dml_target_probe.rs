//! Engine integration tests: a simple `UPDATE` / `DELETE` collects its target rows through
//! the read path's index selection (`Plan::Update` / `Plan::Delete`'s `target_probe`).
//!
//! The differential fixtures (`dml_probe/`) hold the answers — row order, error precedence,
//! the shapes the probe must decline — but an answer cannot show *how* the rows were found:
//! a write that scanned the table answers exactly as one that probed. These tests count the
//! oracle store's full scans instead, so a probe path that stops being taken reddens here
//! and nowhere else. The stripped (`--no-default-features`) build plans no probe, so its
//! expected count is the scan's.

use crate::common;

#[cfg(test)]
mod probe_cost {
    use super::common::*;
    use pglet_sql::executor::{EvalEnv, QueryResult, execute_with_storage};
    use pglet_sql::parser::parse_single;
    use pglet_sql::types::Value;

    /// The full scans a write over an indexed key costs: none when the probe collects the
    /// targets, one (the target's) on the stripped build.
    const PROBED: usize = if cfg!(feature = "index_acceleration") {
        0
    } else {
        1
    };

    /// Run `sql` and return its result and how many full scans it cost.
    fn scans_for(db: &mut TestDb, sql: &str) -> (QueryResult, usize) {
        let before = db.storage.scan_count();
        let result = db.run(sql);
        (result, db.storage.scan_count() - before)
    }

    /// Run `sql` with bound `params` and return its outcome and how many full scans it cost.
    fn scans_for_bound(
        db: &mut TestDb,
        sql: &str,
        params: &[Option<Value>],
    ) -> (pglet_sql::Result<QueryResult>, usize) {
        let plan = parse_single(sql, &db.schema)
            .unwrap_or_else(|e| panic!("{sql}: {e}"))
            .expect("a statement");
        let before = db.storage.scan_count();
        let result = execute_with_storage(
            &plan,
            &mut db.schema,
            &mut db.storage,
            EvalEnv::with_params(params),
            &mut db.sequence_session,
        );
        (result, db.storage.scan_count() - before)
    }

    fn keyed_table(db: &mut TestDb) {
        db.run("CREATE TABLE t (id INT PRIMARY KEY, v TEXT)");
        db.run("INSERT INTO t VALUES (1, 'a'), (2, 'b'), (3, 'c'), (4, 'd'), (5, 'e')");
    }

    fn values_of(db: &mut TestDb, sql: &str) -> Vec<Vec<Value>> {
        db.run(sql).rows
    }

    #[test]
    fn a_keyed_write_reads_its_rows_through_the_index() {
        let mut db = TestDb::new();
        keyed_table(&mut db);
        for (sql, tag) in [
            ("UPDATE t SET v = 'x' WHERE id = 3", "UPDATE 1"),
            (
                "UPDATE t SET v = 'y' WHERE id = 4 AND v IS NOT NULL",
                "UPDATE 1",
            ),
            ("UPDATE t SET v = 'z' WHERE id = 2 AND v <> 'q'", "UPDATE 1"),
            ("UPDATE t SET v = 'z' WHERE id = 1", "UPDATE 1"),
            ("UPDATE t SET v = 'w' WHERE id = 1 AND v = 'z'", "UPDATE 1"),
            ("DELETE FROM t WHERE id = 5", "DELETE 1"),
            ("DELETE FROM t WHERE 4 = id RETURNING v", "DELETE 1"),
        ] {
            let (result, scans) = scans_for(&mut db, sql);
            assert_eq!(result.command_tag, tag, "{sql}");
            assert_eq!(scans, PROBED, "{sql}: full scans");
        }
        assert_eq!(
            values_of(&mut db, "SELECT id, v FROM t ORDER BY id"),
            vec![
                vec![Value::Int4(1), Value::Text("w".into())],
                vec![Value::Int4(2), Value::Text("z".into())],
                vec![Value::Int4(3), Value::Text("x".into())],
            ]
        );
    }

    #[test]
    fn a_bound_key_probes_and_an_unbound_one_declines() {
        let mut db = TestDb::new();
        keyed_table(&mut db);
        let (result, scans) = scans_for_bound(
            &mut db,
            "UPDATE t SET v = 'p' WHERE id = $1",
            &[Some(Value::Int4(2))],
        );
        assert_eq!(result.expect("bound").command_tag, "UPDATE 1");
        assert_eq!(scans, PROBED);
        let (result, scans) = scans_for_bound(
            &mut db,
            "DELETE FROM t WHERE id = $1 AND v = $2",
            &[Some(Value::Int4(2)), Some(Value::Text("p".into()))],
        );
        assert_eq!(result.expect("bound").command_tag, "DELETE 1");
        assert_eq!(scans, PROBED);
        // A NULL key matches no row — through the probe as through the scan.
        let (result, scans) = scans_for_bound(&mut db, "DELETE FROM t WHERE id = $1", &[None]);
        assert_eq!(result.expect("bound").command_tag, "DELETE 0");
        assert_eq!(scans, PROBED);
        // Unbound, the key cannot be evaluated: the probe declines and the scan raises the
        // parameter's error where it reaches it, as it did before there was a probe.
        let (result, scans) = scans_for_bound(&mut db, "UPDATE t SET v = 'q' WHERE id = $1", &[]);
        assert!(result.is_err(), "an unbound key is refused");
        assert_eq!(scans, 1, "the unbound key declines to the scan");
        // The same for a parameter in the residual, which the probe would otherwise skip
        // over every row it leaves out.
        let (result, scans) = scans_for_bound(
            &mut db,
            "UPDATE t SET v = 'q' WHERE v = $2 AND id = $1",
            &[Some(Value::Int4(1))],
        );
        assert!(result.is_err(), "an unbound residual parameter is refused");
        assert_eq!(
            scans, 1,
            "the unbound residual parameter declines to the scan"
        );
    }

    #[test]
    fn an_unindexed_or_uncertain_where_scans_on_both_builds() {
        let mut db = TestDb::new();
        keyed_table(&mut db);
        db.run("CREATE TABLE u (id INT, v TEXT)");
        db.run("INSERT INTO u VALUES (1, 'a'), (2, 'b')");
        for sql in [
            // No index on the key.
            "UPDATE u SET v = 'x' WHERE id = 1",
            "DELETE FROM u WHERE id = 2",
            // A residual conjunct whose evaluation the probe could skip.
            "UPDATE t SET v = 'x' WHERE id = 1 AND v || '' = 'a'",
            "DELETE FROM t WHERE id = 9 AND length(v) > 0",
            // No conjunct a probe can lift.
            "UPDATE t SET v = 'x' WHERE id = 1 OR id = 2",
            // A range, which the read path would probe but a write does not: the
            // in-transaction range lookup reads the table anyway.
            "UPDATE t SET v = 'y' WHERE id > 3",
            "DELETE FROM t WHERE id BETWEEN 7 AND 8 AND v IS NOT NULL",
        ] {
            let (_, scans) = scans_for(&mut db, sql);
            assert_eq!(scans, 1, "{sql}: full scans");
        }
    }

    #[test]
    fn the_probed_targets_are_visited_in_row_id_order() {
        // One key's rows are written in the scan's order, which `RETURNING` shows.
        let mut db = TestDb::new();
        db.run("CREATE TABLE o (k INT, v TEXT)");
        db.run("CREATE INDEX o_k ON o (k)");
        db.run("INSERT INTO o VALUES (1, 'c'), (1, 'a'), (2, 'x'), (1, 'b')");
        let (result, scans) = scans_for(&mut db, "UPDATE o SET v = v WHERE k = 1 RETURNING v");
        assert_eq!(
            result.rows,
            vec![
                vec![Value::Text("c".into())],
                vec![Value::Text("a".into())],
                vec![Value::Text("b".into())]
            ]
        );
        assert_eq!(scans, PROBED);
        // A per-row unique check sees the writes in that order too: `2 → 3` then `1 → 2`
        // succeeds, where the other order would write `1 → 2` over the live `2`.
        db.run("CREATE TABLE q (g INT, k INT UNIQUE)");
        db.run("CREATE INDEX q_g ON q (g)");
        db.run("INSERT INTO q VALUES (1, 2), (1, 1)");
        let (result, scans) = scans_for(&mut db, "UPDATE q SET k = k + 1 WHERE g = 1");
        assert_eq!(result.command_tag, "UPDATE 2");
        assert_eq!(scans, PROBED);
    }
}
