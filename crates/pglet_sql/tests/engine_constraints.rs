//! Engine integration tests: NOT NULL and CHECK constraints.

use crate::common;
use common::*;
use pglet_sql::types::Value;

#[test]
fn test_not_null_rejects_explicit_null_insert() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, name TEXT NOT NULL)");

    let err = db
        .try_run("INSERT INTO t (id, name) VALUES (1, NULL)")
        .expect_err("explicit NULL into NOT NULL column must be rejected");
    assert!(matches!(err, pglet_sql::Error::NotNullViolation { .. }));
}

#[test]
fn test_not_null_rejects_omitted_no_default_insert() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, name TEXT NOT NULL)");

    // Omitting a NOT NULL column with no DEFAULT resolves to NULL → violation.
    let err = db
        .try_run("INSERT INTO t (id) VALUES (1)")
        .expect_err("omitted NOT NULL column with no default must be rejected");
    assert!(matches!(err, pglet_sql::Error::NotNullViolation { .. }));
}

#[test]
fn test_not_null_with_default_omitted_ok() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, name TEXT NOT NULL DEFAULT 'anon')");

    // The DEFAULT fills the omitted column, so NOT NULL is satisfied.
    db.run("INSERT INTO t (id) VALUES (1)");
    let result = db.run("SELECT name FROM t WHERE id = 1");
    assert_eq!(result.rows[0][0], Value::Text("anon".to_string()));
}

#[test]
fn test_not_null_rejects_update_to_null() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, name TEXT NOT NULL)");
    db.run("INSERT INTO t (id, name) VALUES (1, 'alice')");

    let err = db
        .try_run("UPDATE t SET name = NULL WHERE id = 1")
        .expect_err("UPDATE setting a NOT NULL column to NULL must be rejected");
    assert!(matches!(err, pglet_sql::Error::NotNullViolation { .. }));

    // A non-null update still works.
    db.run("UPDATE t SET name = 'bob' WHERE id = 1");
    let result = db.run("SELECT name FROM t WHERE id = 1");
    assert_eq!(result.rows[0][0], Value::Text("bob".to_string()));
}

#[test]
fn test_check_rejects_violating_insert() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, level TEXT CHECK (level IN ('viewer', 'editor')))");

    let err = db
        .try_run("INSERT INTO t (id, level) VALUES (1, 'admin')")
        .expect_err("value outside the CHECK set must be rejected");
    assert!(matches!(err, pglet_sql::Error::CheckViolation { .. }));
}

#[test]
fn test_check_allows_passing_insert() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, level TEXT CHECK (level IN ('viewer', 'editor')))");

    db.run("INSERT INTO t (id, level) VALUES (1, 'editor')");
    let result = db.run("SELECT level FROM t WHERE id = 1");
    assert_eq!(result.rows[0][0], Value::Text("editor".to_string()));
}

#[test]
fn test_check_null_predicate_passes() {
    // SQL 3-valued logic: a CHECK that evaluates to NULL (unknown) PASSES.
    // `level` is nullable, so `level IN (...)` over a NULL `level` is NULL → pass.
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, level TEXT CHECK (level IN ('viewer', 'editor')))");

    db.run("INSERT INTO t (id, level) VALUES (1, NULL)");
    db.run("INSERT INTO t (id) VALUES (2)"); // omitted → NULL → still passes
    let result = db.run("SELECT id FROM t ORDER BY id");
    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_check_rejects_violating_update() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, level TEXT CHECK (level IN ('viewer', 'editor')))");
    db.run("INSERT INTO t (id, level) VALUES (1, 'viewer')");

    let err = db
        .try_run("UPDATE t SET level = 'admin' WHERE id = 1")
        .expect_err("UPDATE violating the CHECK must be rejected");
    assert!(matches!(err, pglet_sql::Error::CheckViolation { .. }));
}

#[test]
fn test_check_multi_column_table_constraint() {
    // A table-level CHECK spanning two columns (the spine's fact-store shape:
    // at least one of two columns must be non-null).
    let mut db = TestDb::new();
    db.run(
        "CREATE TABLE t (id INT, bytes TEXT, external_url TEXT, \
         CHECK (bytes IS NOT NULL OR external_url IS NOT NULL))",
    );

    // One side present → passes.
    db.run("INSERT INTO t (id, bytes) VALUES (1, 'abc')");
    db.run("INSERT INTO t (id, external_url) VALUES (2, 'http://x')");

    // Both null → violation.
    let err = db
        .try_run("INSERT INTO t (id) VALUES (3)")
        .expect_err("both columns null must violate the multi-column CHECK");
    assert!(matches!(err, pglet_sql::Error::CheckViolation { .. }));
}

#[test]
fn test_check_named_constraint_uses_declared_name() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, n INT, CONSTRAINT n_positive CHECK (n > 0))");

    let err = db
        .try_run("INSERT INTO t (id, n) VALUES (1, -5)")
        .expect_err("negative n must violate the named CHECK");
    match err {
        pglet_sql::Error::CheckViolation { constraint, .. } => {
            assert_eq!(constraint, "n_positive");
        }
        other => panic!("expected CheckViolation, got {other:?}"),
    }
}

#[test]
fn test_check_persisted_in_catalog() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, level TEXT CHECK (level IN ('viewer', 'editor')))");

    // The check rides the catalog `Table` so it survives schema round-trips.
    let table = db.schema.get_table("t").expect("table exists");
    assert_eq!(table.checks.len(), 1);
    assert!(
        format!("{:?}", table.checks[0].expr).contains("level"),
        "check predicate references `level`: {:?}",
        table.checks[0].expr
    );
}

// The executor half of the `ErrorResponse` detail fields. The renderer's own rules are
// unit-tested in `pglet_core::error_fields`, and the wire shape in `pglet_server`; what
// only these can catch is the executor **failing to fill** the payload — an omission the
// differential harness is blind to by design, because it scores a missing field as
// agreement (see `ErrorFields::divergences`).

#[test]
fn a_not_null_violation_carries_the_whole_assembled_row() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT NOT NULL, name TEXT)");
    let err = db
        .try_run("INSERT INTO t (name) VALUES ('a')")
        .expect_err("the omitted NOT NULL column");
    // Not the half-built row the per-column check could have shown: the trailing
    // `name` column is present, which is only true after assembly.
    assert_eq!(
        err.detail().as_deref(),
        Some("Failing row contains (null, a)."),
        "{err:?}"
    );
    let loc = err.object_location();
    assert_eq!(loc.table.as_deref(), Some("t"));
    assert_eq!(loc.column.as_deref(), Some("id"));
}

#[test]
fn a_check_violation_carries_the_whole_assembled_row() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT CHECK (a > 0), z TEXT)");
    let err = db
        .try_run("INSERT INTO t VALUES (-1, 'z')")
        .expect_err("the row fails the CHECK");
    assert_eq!(
        err.detail().as_deref(),
        Some("Failing row contains (-1, z)."),
        "{err:?}"
    );
    let loc = err.object_location();
    assert_eq!(loc.table.as_deref(), Some("t"));
    assert_eq!(loc.constraint.as_deref(), Some("t_a_check"));
}

#[test]
fn a_unique_violation_carries_the_table_and_the_colliding_key() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT, b TEXT, UNIQUE (a, b))");
    db.run("INSERT INTO t VALUES (1, 'x')");
    let err = db
        .try_run("INSERT INTO t VALUES (1, 'x')")
        .expect_err("the duplicate composite key");
    assert_eq!(
        err.detail().as_deref(),
        Some("Key (a, b)=(1, x) already exists."),
        "{err:?}"
    );
    // The table is the fact the constraint's name does not determine.
    assert_eq!(err.object_location().table.as_deref(), Some("t"));
}

#[test]
fn a_foreign_key_violation_names_the_child_table_in_both_directions() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE p (id INT PRIMARY KEY)");
    db.run("CREATE TABLE c (a INT REFERENCES p(id))");

    let insert = db
        .try_run("INSERT INTO c VALUES (99)")
        .expect_err("no such parent");
    assert_eq!(
        insert.detail().as_deref(),
        Some("Key (a)=(99) is not present in table \"p\"."),
        "{insert:?}"
    );
    assert_eq!(insert.object_location().table.as_deref(), Some("c"));

    db.run("INSERT INTO p VALUES (1)");
    db.run("INSERT INTO c VALUES (1)");
    let delete = db
        .try_run("DELETE FROM p WHERE id = 1")
        .expect_err("a child still references it");
    assert_eq!(
        delete.detail().as_deref(),
        Some("Key (id)=(1) is still referenced from table \"c\"."),
        "{delete:?}"
    );
    // The message names the parent here; `t` must still name the child.
    assert!(delete.to_string().contains("on table \"p\""), "{delete}");
    assert_eq!(delete.object_location().table.as_deref(), Some("c"));
}

#[test]
fn a_syntax_error_carries_a_codepoint_position_from_the_parser() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT)");
    let err = db
        .try_run("SELECT 'ααααα' AS a, FROM t")
        .expect_err("a trailing comma in the select list");
    // Five two-byte characters: the byte offset would be 26.
    assert_eq!(err.position(), Some(22), "{err:?}");
    assert!(
        !err.to_string().contains("position"),
        "the position must not ride in the message: {err}"
    );
}

// The UPDATE half of the row-scoped NOT NULL move. Making the check row-scoped is
// right; running it where the *written* row's constraints run is not, because that is
// after the foreign-key actions (`FkTriggerQueue`). 18.6 runs `ExecConstraints` on the new tuple inside
// `ExecUpdate` and queues the referential actions as AFTER-ROW triggers, so a null in
// the target outranks every child consequence — measured on 18.6 for all three actions.

#[test]
fn a_target_not_null_outranks_every_referential_action_on_one_update() {
    for (action, extra) in [
        ("RESTRICT", ""),
        ("NO ACTION", ""),
        ("SET NULL", ", CHECK (pid IS NOT NULL)"),
    ] {
        let mut db = TestDb::new();
        db.run("CREATE TABLE p (id INT PRIMARY KEY, v INT NOT NULL)");
        db.run(&format!(
            "CREATE TABLE c (pid INT REFERENCES p(id) ON UPDATE {action}{extra})"
        ));
        db.run("INSERT INTO p VALUES (1, 1)");
        db.run("INSERT INTO c VALUES (1)");
        let err = db
            .try_run("UPDATE p SET id = 2, v = NULL")
            .expect_err("the update nulls a NOT NULL column");
        assert_eq!(err.sqlstate(), "23502", "{action}: {err:?}");
        // And the detail is the post-update target row, not a child's.
        assert_eq!(
            err.detail().as_deref(),
            Some("Failing row contains (2, null)."),
            "{action}: {err:?}"
        );
        assert_eq!(
            err.object_location().table.as_deref(),
            Some("p"),
            "{action}"
        );
    }
}

#[test]
fn a_failed_update_not_null_leaves_no_row_written() {
    // `MemStorage` has no rollback, so a check that runs per written row would leave
    // the rows before the offending one updated — the class CHECK already has here.
    // NOT NULL must not join it: the gate runs over the collected images before the
    // first write.
    let mut db = TestDb::new();
    db.run("CREATE TABLE m (a INT NOT NULL, b INT)");
    db.run("INSERT INTO m VALUES (1, 1), (2, 2)");
    let err = db
        .try_run("UPDATE m SET a = CASE WHEN b = 1 THEN 7 ELSE NULL END")
        .expect_err("the second row nulls a NOT NULL column");
    assert_eq!(err.sqlstate(), "23502", "{err:?}");
    let rows = db.run("SELECT a, b FROM m ORDER BY b").rows;
    assert_eq!(
        rows,
        vec![
            vec![Value::Int4(1), Value::Int4(1)],
            vec![Value::Int4(2), Value::Int4(2)],
        ],
        "no row may be written when the statement fails"
    );
}
