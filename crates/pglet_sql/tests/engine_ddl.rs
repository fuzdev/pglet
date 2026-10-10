//! Engine integration tests: DDL (CREATE/DROP/TRUNCATE TABLE) and ENUM types.

use crate::common;
use common::*;
use pglet_sql::types::{PgNumeric, Value};

#[test]
fn test_create_and_drop_table() {
    let mut db = TestDb::new();

    let result = db.run("CREATE TABLE test (id INT, name TEXT)");
    assert_eq!(result.command_tag, "CREATE TABLE");
    assert!(db.schema.get_table("test").is_some());

    let result = db.run("DROP TABLE test");
    assert_eq!(result.command_tag, "DROP TABLE");
    assert!(db.schema.get_table("test").is_none());
}

#[test]
fn test_create_table_if_not_exists() {
    let mut db = TestDb::new();

    db.run("CREATE TABLE test (id INT)");
    // Second create should succeed due to IF NOT EXISTS
    let result = db.run("CREATE TABLE IF NOT EXISTS test (id INT)");
    assert_eq!(result.command_tag, "CREATE TABLE");
}

#[test]
fn test_drop_table_if_exists() {
    let mut db = TestDb::new();

    // Drop non-existent table with IF EXISTS should succeed
    let result = db.run("DROP TABLE IF EXISTS nonexistent");
    assert_eq!(result.command_tag, "DROP TABLE");
}

#[test]
fn test_truncate_empties_table_but_keeps_it() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, name TEXT)");
    db.run("INSERT INTO t VALUES (1, 'a'), (2, 'b'), (3, 'c')");

    let result = db.run("TRUNCATE t");
    assert_eq!(result.command_tag, "TRUNCATE TABLE");

    assert!(
        db.run("SELECT * FROM t").rows.is_empty(),
        "TRUNCATE must remove every row"
    );
    // Unlike DROP, the table definition survives.
    assert!(db.schema.get_table("t").is_some());
}

#[test]
fn test_truncate_multiple_tables() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE a (id INT)");
    db.run("CREATE TABLE b (id INT)");
    db.run("INSERT INTO a VALUES (1), (2)");
    db.run("INSERT INTO b VALUES (10)");

    db.run("TRUNCATE a, b");

    assert!(db.run("SELECT * FROM a").rows.is_empty());
    assert!(db.run("SELECT * FROM b").rows.is_empty());
}

#[test]
fn test_truncate_cascade_clears_referencing_table() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE parent (id INT PRIMARY KEY)");
    db.run("CREATE TABLE child (id INT, parent_id INT REFERENCES parent(id))");
    db.run("INSERT INTO parent VALUES (1)");
    db.run("INSERT INTO child VALUES (10, 1)");

    // CASCADE extends truncation to the FK-referencing child.
    db.run("TRUNCATE parent CASCADE");

    assert!(db.run("SELECT * FROM parent").rows.is_empty());
    assert!(
        db.run("SELECT * FROM child").rows.is_empty(),
        "CASCADE must also clear the referencing table"
    );
}

#[test]
fn test_truncate_referenced_without_cascade_fails_loud() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE parent (id INT PRIMARY KEY)");
    db.run("CREATE TABLE child (id INT, parent_id INT REFERENCES parent(id))");
    db.run("INSERT INTO parent VALUES (1)");
    db.run("INSERT INTO child VALUES (10, 1)");

    // Without CASCADE, truncating an FK-referenced table must error rather than
    // silently orphan the child's referencing rows.
    let err = db
        .try_run("TRUNCATE parent")
        .expect_err("truncating an FK-referenced table without CASCADE must error");
    let msg = err.to_string();
    assert!(msg.contains("CASCADE"), "expected the FK guard, got: {msg}");

    // The rejected TRUNCATE left both tables untouched.
    assert_eq!(db.run("SELECT * FROM parent").rows.len(), 1);
    assert_eq!(db.run("SELECT * FROM child").rows.len(), 1);
}

#[test]
fn test_truncate_restart_identity_resets_serial() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id SERIAL, v TEXT)");
    db.run("INSERT INTO t (v) VALUES ('a'), ('b')"); // id 1, 2

    db.run("TRUNCATE t RESTART IDENTITY");
    db.run("INSERT INTO t (v) VALUES ('c')"); // sequence reset -> id 1

    let rows = db.run("SELECT id FROM t");
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0][0], Value::Int4(1));
}

#[test]
fn test_truncate_default_continues_identity() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id SERIAL, v TEXT)");
    db.run("INSERT INTO t (v) VALUES ('a'), ('b')"); // id 1, 2

    // Default (CONTINUE IDENTITY) leaves the sequence alone.
    db.run("TRUNCATE t");
    db.run("INSERT INTO t (v) VALUES ('c')"); // sequence keeps counting -> id 3

    let rows = db.run("SELECT id FROM t");
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0][0], Value::Int4(3));
}

#[test]
fn test_truncate_nonexistent_table_errors() {
    let mut db = TestDb::new();
    assert!(
        db.try_run("TRUNCATE nope").is_err(),
        "truncating a missing table must error"
    );
}

#[test]
fn test_alter_add_check_enforces_future_writes() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT, b INT)");
    db.run("ALTER TABLE t ADD CONSTRAINT t_ab CHECK (a < b)");

    db.run("INSERT INTO t VALUES (1, 2)"); // satisfies a < b
    let err = db
        .try_run("INSERT INTO t VALUES (5, 2)")
        .expect_err("a < b is violated");
    assert!(matches!(err, pglet_sql::Error::CheckViolation { .. }));
}

#[test]
fn test_alter_add_check_validates_existing_rows() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT, b INT)");
    db.run("INSERT INTO t VALUES (5, 2)"); // already violates a < b

    let err = db
        .try_run("ALTER TABLE t ADD CONSTRAINT t_ab CHECK (a < b)")
        .expect_err("existing row violates the new CHECK");
    // The ALTER-time validation scan is 18.6's `ATRewriteTable` route, not the
    // write-path `ExecConstraints` one: a different message and no `Failing row`
    // detail, even though both are `23514`.
    assert!(
        matches!(err, pglet_sql::Error::CheckConstraintNotSatisfied { .. }),
        "{err:?}"
    );

    // The rejected ADD left the catalog untouched: had the check been recorded, a
    // (9, 1) row would be rejected — it isn't.
    db.run("DELETE FROM t");
    db.run("INSERT INTO t VALUES (9, 1)");
}

#[test]
fn test_alter_add_constraint_duplicate_name_loud() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT, b INT, CONSTRAINT t_pos CHECK (a > 0))");

    // A named ADD CONSTRAINT whose name already belongs to a constraint on the table
    // is 42710 (duplicate_object) in Postgres — not a silent second copy.
    let err = db
        .try_run("ALTER TABLE t ADD CONSTRAINT t_pos CHECK (b > 0)")
        .expect_err("duplicate constraint name must be rejected");
    assert!(
        matches!(
            err,
            pglet_sql::Error::Catalog(pglet_sql::CatalogError::ConstraintAlreadyExists { .. })
        ),
        "expected ConstraintAlreadyExists, got {err:?}"
    );
    assert_eq!(err.sqlstate(), "42710");

    // The rejected ADD left the catalog untouched: the original `a > 0` still holds
    // and `b > 0` was NOT recorded — a (1, -1) row inserts.
    db.run("INSERT INTO t VALUES (1, -1)");

    // A distinct name is accepted (no false positive), and an unnamed ADD CHECK has
    // no name to collide.
    db.run("ALTER TABLE t ADD CONSTRAINT t_b_pos CHECK (b < 100)");
    db.run("ALTER TABLE t ADD CHECK (a < 100)");
}

#[test]
fn test_alter_add_constraint_name_collides_with_fk_name() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE parent (id INT PRIMARY KEY)");
    // The FK gets the explicit constraint name `dup` on the child.
    db.run("CREATE TABLE child (id INT, parent_id INT, CONSTRAINT dup FOREIGN KEY (parent_id) REFERENCES parent(id))");

    // A CHECK reusing the FK's name collides (Postgres shares one per-table
    // constraint namespace across CHECK and FK).
    let err = db
        .try_run("ALTER TABLE child ADD CONSTRAINT dup CHECK (id > 0)")
        .expect_err("constraint name shared with an FK must be rejected");
    assert_eq!(err.sqlstate(), "42710");
}

#[test]
fn test_alter_add_constraint_foreign_key_works_and_enforces() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE parent (id INT PRIMARY KEY)");
    db.run("INSERT INTO parent VALUES (1), (2)");
    db.run("CREATE TABLE child (id INT, parent_id INT)");
    db.run("INSERT INTO child VALUES (1, 1)"); // references a live parent

    // Single-column FK via ALTER ADD CONSTRAINT builds a real FK (the existing child
    // row validated), then enforces: an orphan insert is rejected (23503).
    db.run(
        "ALTER TABLE child ADD CONSTRAINT child_fk FOREIGN KEY (parent_id) REFERENCES parent(id)",
    );
    let err = db
        .try_run("INSERT INTO child VALUES (2, 99)")
        .expect_err("orphan child must be rejected by the new FK");
    assert!(matches!(err, pglet_sql::Error::ForeignKeyViolation { .. }));
    assert_eq!(err.sqlstate(), "23503");
}

#[test]
fn test_alter_add_constraint_foreign_key_validates_existing_rows() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE parent (id INT PRIMARY KEY)");
    db.run("CREATE TABLE child (id INT, parent_id INT)");
    db.run("INSERT INTO child VALUES (1, 42)"); // 42 has no parent

    // ADD CONSTRAINT FK rejects when an existing child row is already an orphan.
    let err = db
        .try_run("ALTER TABLE child ADD CONSTRAINT child_fk FOREIGN KEY (parent_id) REFERENCES parent(id)")
        .expect_err("existing orphan row violates the new FK");
    assert_eq!(err.sqlstate(), "23503");
}

#[test]
fn test_alter_add_constraint_composite_foreign_key_works_and_enforces() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE parent (x INT, y INT, PRIMARY KEY (x, y))");
    db.run("INSERT INTO parent VALUES (1, 2)");
    db.run("CREATE TABLE child (a INT, b INT)");
    db.run("INSERT INTO child VALUES (1, 2)"); // references the live (1, 2) parent

    // A composite (multi-column) FK via ALTER ADD CONSTRAINT builds a real FK (the
    // existing child tuple validated), then enforces: an orphan insert is rejected.
    db.run("ALTER TABLE child ADD CONSTRAINT child_fk FOREIGN KEY (a, b) REFERENCES parent(x, y)");
    let err = db
        .try_run("INSERT INTO child VALUES (3, 4)")
        .expect_err("orphan composite child must be rejected by the new FK");
    assert_eq!(err.sqlstate(), "23503");
    // A NULL in any FK column exempts the row (MATCH SIMPLE).
    db.run("INSERT INTO child VALUES (9, NULL)");
}

#[test]
fn test_bad_fk_create_table_leaves_schema_untouched() {
    // A CREATE TABLE that fails its FK validation must leave the catalog byte-clean:
    // no phantom half-created table (the autocommit path does not restore the schema
    // on a statement error — validation runs BEFORE any mutation).
    let mut db = TestDb::new();

    // 42830: the referenced (x, y) has no covering unique/PK key on the parent.
    db.run("CREATE TABLE nu_parent (x INT, y INT)");
    let err = db
        .try_run(
            "CREATE TABLE nu_child (a INT, b INT, FOREIGN KEY (a, b) REFERENCES nu_parent (x, y))",
        )
        .expect_err("FK to a non-unique referenced key must fail");
    assert_eq!(err.sqlstate(), "42830");
    assert!(
        db.schema.get_table("nu_child").is_none(),
        "the failed CREATE must NOT leave a phantom table in the schema"
    );
    // Proof the name is truly free: a re-CREATE of the same name succeeds (would be
    // 42P07 "already exists" if the phantom had leaked).
    db.run("CREATE TABLE nu_child (a INT, b INT)");

    // 42P01: the referenced parent table does not exist.
    let err = db
        .try_run("CREATE TABLE mp_child (a INT, FOREIGN KEY (a) REFERENCES no_such_parent (id))")
        .expect_err("FK to a missing parent must fail");
    assert_eq!(err.sqlstate(), "42P01");
    assert!(db.schema.get_table("mp_child").is_none());
    db.run("CREATE TABLE mp_child (a INT)");
}

#[test]
fn test_bad_inline_fk_add_column_leaves_schema_untouched() {
    // ALTER TABLE ... ADD COLUMN ... REFERENCES <missing> must not leak the column.
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT PRIMARY KEY)");
    let err = db
        .try_run("ALTER TABLE t ADD COLUMN c INT REFERENCES no_such_parent (id)")
        .expect_err("ADD COLUMN with an FK to a missing parent must fail");
    assert_eq!(err.sqlstate(), "42P01");
    assert!(
        !db.schema
            .get_table("t")
            .expect("t exists")
            .columns
            .iter()
            .any(|col| col.name == "c"),
        "the failed ADD COLUMN must NOT leave a phantom column"
    );
    // The name is free — re-adding it (without the bad FK) succeeds.
    db.run("ALTER TABLE t ADD COLUMN c INT");
}

#[test]
fn test_self_referential_composite_fk_create_ok() {
    // A valid self-referential composite FK still creates cleanly: the parent (this
    // same table) isn't in the catalog yet, so the covering-key check runs against the
    // in-flight PRIMARY KEY.
    let mut db = TestDb::new();
    db.run(
        "CREATE TABLE tree (a INT, b INT, pa INT, pb INT, PRIMARY KEY (a, b), \
         FOREIGN KEY (pa, pb) REFERENCES tree (a, b))",
    );
    db.run("INSERT INTO tree VALUES (1, 1, NULL, NULL)"); // root — MATCH SIMPLE exempts NULLs
    db.run("INSERT INTO tree VALUES (2, 2, 1, 1)"); // references the live (1, 1)
    let err = db
        .try_run("INSERT INTO tree VALUES (3, 3, 9, 9)")
        .expect_err("child pointing at a missing (9, 9) parent must be rejected");
    assert_eq!(err.sqlstate(), "23503");
}

#[test]
fn test_foreign_key_match_simple_ok_full_and_partial_loud() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE parent (x INT, y INT, PRIMARY KEY (x, y))");
    // MATCH SIMPLE is the default pglet implements — accepted.
    db.run(
        "CREATE TABLE c1 (a INT, b INT, FOREIGN KEY (a, b) REFERENCES parent (x, y) MATCH SIMPLE)",
    );
    // MATCH FULL / MATCH PARTIAL change the NULL semantics — pglet fails loud (0A000)
    // rather than silently treating them as SIMPLE.
    for spelling in ["MATCH FULL", "MATCH PARTIAL"] {
        let sql = format!(
            "CREATE TABLE cx (a INT, b INT, FOREIGN KEY (a, b) REFERENCES parent (x, y) {spelling})"
        );
        let err = db
            .try_run(&sql)
            .expect_err("MATCH FULL / PARTIAL must be rejected");
        assert!(matches!(err, pglet_sql::Error::NotImplemented(_)));
        assert_eq!(err.sqlstate(), "0A000");
    }
}

#[test]
fn test_alter_add_constraint_composite_foreign_key_no_unique_is_loud() {
    let mut db = TestDb::new();
    // No unique/PK constraint covering parent(a, b) → 42830 at ADD CONSTRAINT.
    db.run("CREATE TABLE parent (a INT, b INT)");
    db.run("CREATE TABLE child (a INT, b INT)");
    let err = db
        .try_run(
            "ALTER TABLE child ADD CONSTRAINT child_fk FOREIGN KEY (a, b) REFERENCES parent(a, b)",
        )
        .expect_err("FK to a non-unique referenced key must be rejected");
    assert_eq!(err.sqlstate(), "42830");
}

#[test]
fn test_alter_add_constraint_primary_key_works_and_enforces() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, name TEXT)");
    db.run("INSERT INTO t VALUES (1, 'a'), (2, 'b')");

    // PRIMARY KEY via ALTER ADD CONSTRAINT builds an enforced unique index over the
    // (validated) existing rows; a later duplicate is rejected (23505).
    db.run("ALTER TABLE t ADD CONSTRAINT t_pkey PRIMARY KEY (id)");
    let err = db
        .try_run("INSERT INTO t VALUES (1, 'dup')")
        .expect_err("duplicate id must be rejected by the new PK");
    assert!(matches!(err, pglet_sql::Error::UniqueViolation { .. }));
    assert_eq!(err.sqlstate(), "23505");

    // The PK also made `id` NOT NULL.
    let err = db
        .try_run("INSERT INTO t (name) VALUES ('c')")
        .expect_err("a PK column is NOT NULL");
    assert_eq!(err.sqlstate(), "23502");
}

#[test]
fn test_alter_add_constraint_primary_key_validates_existing_rows() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT)");
    db.run("INSERT INTO t VALUES (1), (1)"); // already duplicated

    let err = db
        .try_run("ALTER TABLE t ADD CONSTRAINT t_pkey PRIMARY KEY (id)")
        .expect_err("existing duplicate violates the new PK");
    assert_eq!(err.sqlstate(), "23505");

    // A NULL in an existing row violates the PK's implied NOT NULL.
    let mut db2 = TestDb::new();
    db2.run("CREATE TABLE t (id INT)");
    db2.run("INSERT INTO t VALUES (NULL)");
    let err = db2
        .try_run("ALTER TABLE t ADD CONSTRAINT t_pkey PRIMARY KEY (id)")
        .expect_err("existing NULL violates the new PK");
    assert_eq!(err.sqlstate(), "23502");
}

#[test]
fn test_alter_set_not_null_enforces() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, name TEXT)");
    db.run("INSERT INTO t VALUES (1, 'a'), (2, 'b')");

    // SET NOT NULL over a column with no existing NULLs succeeds and is enforced
    // (not a silent no-op): a later NULL insert is rejected with 23502.
    let result = db.run("ALTER TABLE t ALTER COLUMN name SET NOT NULL");
    assert_eq!(result.command_tag, "ALTER TABLE");
    let err = db
        .try_run("INSERT INTO t (id) VALUES (3)")
        .expect_err("a NULL into the now-NOT NULL column must be rejected");
    assert_eq!(err.sqlstate(), "23502");
}

#[test]
fn test_alter_set_not_null_validates_existing_rows() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, name TEXT)");
    db.run("INSERT INTO t VALUES (1, NULL)");

    // An existing NULL blocks SET NOT NULL (23502); validate-before-mutate leaves the
    // catalog unchanged, so a subsequent NULL insert still succeeds.
    let err = db
        .try_run("ALTER TABLE t ALTER COLUMN name SET NOT NULL")
        .expect_err("an existing NULL must block SET NOT NULL");
    assert_eq!(err.sqlstate(), "23502");
    db.run("INSERT INTO t VALUES (2, NULL)"); // still nullable — no error
}

#[test]
fn test_alter_drop_not_null_and_type_change_fail_loud() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT NOT NULL, name TEXT)");

    // Unimplemented, semantically-meaningful column actions fail loud (0A000) rather
    // than silently no-op (the correct-or-loud contract).
    for sql in [
        "ALTER TABLE t ALTER COLUMN id DROP NOT NULL",
        "ALTER TABLE t ALTER COLUMN id TYPE BIGINT",
        "ALTER TABLE t ALTER COLUMN id SET DATA TYPE BIGINT",
    ] {
        let err = db.try_run(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), "0A000", "{sql}");
    }

    // A cosmetic SET stays a tolerated no-op (no data effect).
    let result = db.run("ALTER TABLE t ALTER COLUMN name SET STATISTICS 100");
    assert_eq!(result.command_tag, "ALTER TABLE");
}

#[test]
fn test_schema_qualified_sequence_key_is_consistent() {
    let mut db = TestDb::new();
    // `CREATE SEQUENCE public.s` reduces its catalog key to `s`; a schema-qualified
    // `nextval('public.s')` must resolve the SAME slot, so `START WITH` is honored.
    // (Before the key-normalization fix the two kept distinct `s` / `public.s` slots,
    // the seeded start was silently lost, and nextval began at 1 — pg_dump always
    // emits the qualified `nextval('public.<seq>'::regclass)`.)
    db.run("CREATE SEQUENCE public.s START WITH 100");
    db.run("CREATE TABLE t (id INT DEFAULT nextval('public.s'::regclass), v TEXT)");
    db.run("INSERT INTO t (v) VALUES ('a')");
    let result = db.run("SELECT id FROM t");
    assert_eq!(result.rows[0][0], Value::Int4(100));
}

#[test]
fn test_alter_add_constraint_unique_works_and_enforces() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, email TEXT)");
    db.run("INSERT INTO t VALUES (1, 'a@x'), (2, 'b@x')");

    db.run("ALTER TABLE t ADD CONSTRAINT t_email_key UNIQUE (email)");
    let err = db
        .try_run("INSERT INTO t VALUES (3, 'a@x')")
        .expect_err("duplicate email must be rejected by the new UNIQUE");
    assert_eq!(err.sqlstate(), "23505");

    // NULLs are distinct, so two NULL emails coexist.
    db.run("INSERT INTO t VALUES (4, NULL)");
    db.run("INSERT INTO t VALUES (5, NULL)");
}

#[test]
fn test_alter_set_default_nextval_works() {
    let mut db = TestDb::new();
    db.run("CREATE SEQUENCE s START WITH 10");
    db.run("CREATE TABLE t (id INT NOT NULL, name TEXT)");
    db.run("ALTER TABLE ONLY t ALTER COLUMN id SET DEFAULT nextval('s')");

    // The re-attached default resolves the registered sequence.
    db.run("INSERT INTO t (name) VALUES ('a')");
    db.run("INSERT INTO t (name) VALUES ('b')");
    let result = db.run("SELECT id, name FROM t ORDER BY id");
    assert_eq!(result.rows[0][0], Value::Int4(10));
    assert_eq!(result.rows[1][0], Value::Int4(11));

    // DROP DEFAULT clears it: an omitted NOT NULL column then fails loud.
    db.run("ALTER TABLE t ALTER COLUMN id DROP DEFAULT");
    let err = db
        .try_run("INSERT INTO t (name) VALUES ('c')")
        .expect_err("no default after DROP DEFAULT on a NOT NULL column");
    assert_eq!(err.sqlstate(), "23502");
}

#[test]
fn test_enum_default_and_explicit_members() {
    let mut db = TestDb::new();
    setup_enum(&mut db);

    // Omitted column takes the enum DEFAULT; an explicit member inserts.
    db.run("INSERT INTO cell (id) VALUES (1)");
    db.run("INSERT INTO cell (id, visibility) VALUES (2, 'public')");

    let result = db.run("SELECT id, visibility FROM cell ORDER BY id");
    assert_eq!(result.rows.len(), 2);
    // Enum values round-trip as their text label.
    assert_eq!(result.rows[0][1], Value::Text("private".to_string()));
    assert_eq!(result.rows[1][1], Value::Text("public".to_string()));
}

#[test]
fn test_enum_rejects_invalid_insert() {
    let mut db = TestDb::new();
    setup_enum(&mut db);

    let err = db
        .try_run("INSERT INTO cell (id, visibility) VALUES (1, 'bogus')")
        .expect_err("non-member value must be rejected");
    assert!(matches!(err, pglet_sql::Error::InvalidEnumValue { .. }));
}

#[test]
fn test_enum_rejects_invalid_update() {
    let mut db = TestDb::new();
    setup_enum(&mut db);
    db.run("INSERT INTO cell (id) VALUES (1)");

    let err = db
        .try_run("UPDATE cell SET visibility = 'bogus' WHERE id = 1")
        .expect_err("non-member value must be rejected");
    assert!(matches!(err, pglet_sql::Error::InvalidEnumValue { .. }));

    // A valid member updates fine.
    db.run("UPDATE cell SET visibility = 'public' WHERE id = 1");
    let result = db.run("SELECT visibility FROM cell WHERE id = 1");
    assert_eq!(result.rows[0][0], Value::Text("public".to_string()));
}

#[test]
fn test_enum_rejects_invalid_upsert_update() {
    let mut db = TestDb::new();
    setup_enum(&mut db);
    db.run("INSERT INTO cell (id) VALUES (1)");

    // ON CONFLICT DO UPDATE must reject a non-member enum just as plain INSERT
    // and plain UPDATE do (the post-update row flows through the same per-column
    // validation).
    let err = db
        .try_run(
            "INSERT INTO cell (id, visibility) VALUES (1, 'public') \
             ON CONFLICT (id) DO UPDATE SET visibility = 'bogus'",
        )
        .expect_err("non-member value must be rejected on upsert update");
    assert!(matches!(err, pglet_sql::Error::InvalidEnumValue { .. }));

    // A valid member upserts fine.
    db.run(
        "INSERT INTO cell (id, visibility) VALUES (1, 'public') \
         ON CONFLICT (id) DO UPDATE SET visibility = 'public'",
    );
    let result = db.run("SELECT visibility FROM cell WHERE id = 1");
    assert_eq!(result.rows[0][0], Value::Text("public".to_string()));
}

#[test]
fn test_enum_equality_filter() {
    let mut db = TestDb::new();
    setup_enum(&mut db);
    db.run("INSERT INTO cell (id, visibility) VALUES (1, 'private'), (2, 'public'), (3, 'public')");

    let result = db.run("SELECT id FROM cell WHERE visibility = 'public' ORDER BY id");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Int4(2));
    assert_eq!(result.rows[1][0], Value::Int4(3));
}

#[test]
fn test_cast_to_enum() {
    let mut db = TestDb::new();
    setup_enum(&mut db);
    db.run("INSERT INTO cell (id, visibility) VALUES (1, 'private'), (2, 'public')");

    // A direct cast to the user enum (`'public'::cell_visibility`) and the text
    // round-trip the spine uses (`'public'::text::cell_visibility`) both resolve
    // the enum name against the catalog (enums are stored as their TEXT label)
    // rather than failing loud as an unsupported type. Casting back to text reads
    // the label unchanged, and the casted literal compares equal to the column.
    let result = db.run(
        "SELECT id FROM cell \
         WHERE visibility = 'public'::cell_visibility \
           AND ('public'::text::cell_visibility)::text = 'public' \
         ORDER BY id",
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(2));
}

#[test]
fn test_drop_type() {
    let mut db = TestDb::new();
    db.run("CREATE TYPE cell_visibility AS ENUM ('private', 'public')");
    assert!(db.schema.enums.contains_key("cell_visibility"));

    db.run("DROP TYPE cell_visibility");
    assert!(!db.schema.enums.contains_key("cell_visibility"));

    // IF EXISTS is a no-op when absent; a bare DROP errors.
    db.run("DROP TYPE IF EXISTS cell_visibility");
    assert!(db.try_run("DROP TYPE cell_visibility").is_err());
}

#[test]
fn drop_column_at_or_below_indexed_position_fails_loud() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT, b INT, c INT)");
    db.run("CREATE INDEX t_c_idx ON t (c)"); // keys on row position 2

    // Dropping `a` (position 0) would shift `c`'s stored key position. pglet keys
    // indexes by resolved position (Postgres uses stable attnums) and does not
    // rebuild them on drop, so it must fail loud (0A000) rather than silently key
    // the index on the wrong column.
    let err = db
        .try_run("ALTER TABLE t DROP COLUMN a")
        .expect_err("drop at/below an indexed position must fail loud");
    assert_eq!(
        err.sqlstate(),
        "0A000",
        "expected feature_not_supported, got {err:?}"
    );

    // The rejected drop left the catalog untouched.
    let table = db
        .schema
        .get_table("t")
        .expect("table survives a rejected drop");
    assert!(table.columns.iter().any(|c| c.name == "a"));
    assert_eq!(table.columns.len(), 3);
}

/// The refusal names the reason it actually found, and there are three.
///
/// One sentence stood for all three and asserted the narrowest: over
/// `dcm(a, b, c)` under `CREATE INDEX ON dcm (a) WHERE c > 0`, dropping `b` was refused
/// as *"keys on this column or one after it"* — the key is at position 0, before the
/// drop, so the sentence was false and hid the predicate that is the real reason. (18.6
/// accepts that drop outright; the over-rejection is `EDGES.md`'s, this is the message.)
#[test]
fn the_drop_column_refusal_names_which_of_the_three_reasons_it_found() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE dcm (a INT, b INT, c INT)");
    db.run("CREATE INDEX dcm_key ON dcm (c)");
    db.run("CREATE TABLE dcp (a INT, b INT, c INT)");
    db.run("CREATE INDEX dcp_pred ON dcp (a) WHERE c > 0");
    db.run("CREATE TABLE dce (a INT, b INT, c INT)");
    db.run("CREATE INDEX dce_expr ON dce ((a + c))");

    for (stmt, expected) in [
        (
            "ALTER TABLE dcm DROP COLUMN a",
            "keys on this column or one after it",
        ),
        (
            "ALTER TABLE dcp DROP COLUMN b",
            "has a partial predicate, which reads the row by position",
        ),
        (
            "ALTER TABLE dce DROP COLUMN b",
            "has an expression key, which reads the row by position",
        ),
    ] {
        let err = db.try_run(stmt).expect_err("every one of these is refused");
        assert_eq!(err.sqlstate(), "0A000", "{stmt}: {err:?}");
        let msg = err.to_string();
        assert!(
            msg.contains(expected),
            "{stmt} must name its own reason, got {msg}"
        );
    }
}

#[test]
fn drop_column_after_all_indexed_positions_succeeds() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT, b INT, c INT)");
    db.run("CREATE INDEX t_a_idx ON t (a)"); // keys on row position 0

    // Dropping `c` (position 2, after the only indexed column) leaves the index's
    // key position unchanged, so the drop is allowed.
    db.try_run("ALTER TABLE t DROP COLUMN c")
        .expect("drop of a column after all indexed positions must succeed");
    let table = db.schema.get_table("t").expect("table still present");
    assert!(table.columns.iter().all(|col| col.name != "c"));
    assert_eq!(table.columns.len(), 2);
}

#[test]
fn test_signed_numeric_column_default() {
    // A signed numeric `DEFAULT` — Postgres accepts `DEFAULT -7` / `DEFAULT +7`, and
    // `parse_column_default` was once a token-level classifier that recognized only a
    // bare `Number` token, rejecting any sign as an "unsupported DEFAULT expression".
    // Both signs come through the general expression grammar now, and both still fold
    // into the stored `Value` rather than persisting an `Expr::UnaryPlus` wrapper.
    let mut db = TestDb::new();
    db.run("CREATE TABLE s (a INT, b INT DEFAULT -7, c INT DEFAULT +7, d NUMERIC DEFAULT -1.5)");
    db.run("INSERT INTO s (a) VALUES (5)");
    let rows = db.run("SELECT b, c, d FROM s").rows;
    assert_eq!(rows[0][0], Value::Int4(-7));
    assert_eq!(rows[0][1], Value::Int4(7));
    assert_eq!(
        rows[0][2],
        Value::Numeric(PgNumeric::parse("-1.5").expect("numeric"))
    );
}

#[test]
fn test_signed_non_numeric_default_still_fails_loud() {
    // The sign is consumed **only** when a `Number` actually follows. A signed
    // non-numeric default must therefore still reach the fail-loud arm rather than
    // being half-consumed into a bogus literal — which would store a silently wrong
    // default (and flip whether a NOT NULL column accepts an omitted INSERT).
    //
    // Deliberately stricter than Postgres, which resolves the unknown literal under
    // unary `+` to float8 and fails `22P02` at DDL time; pglet fails `0A000`. Both
    // loud, so no silently-wrong default is stored either way. Not a differential
    // fixture precisely because the SQLSTATEs differ.
    let mut db = TestDb::new();
    db.try_run("CREATE TABLE bad (z INT DEFAULT +'abc')")
        .expect_err("a signed non-numeric DEFAULT is not a recognized default form");
}

/// A label the enum no longer holds still compares, and sorts **after** every member.
///
/// The one route that puts a non-member into a stored row is `DROP TYPE` + re-`CREATE
/// TYPE` with different members — pglet's `DROP TYPE` does no dependency check, where
/// 18.6 raises `2BP01`, so no differential fixture can express this at all. 18.6 offers
/// no answer to copy either: its enum datum *is* a validated `pg_enum.oid`, so a label
/// its cache lacks is an internal `elog(ERROR)` rather than a modelled SQLSTATE. The
/// rule is pglet's own, and it is chosen so a readable table stays readable — the same
/// posture the dump emitter takes toward a sequence owner it cannot resolve.
#[test]
fn test_enum_non_member_label_sorts_after_every_member() {
    let mut db = TestDb::new();
    db.run("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')");
    db.run("CREATE TABLE m3 (id INT, v mood)");
    db.run("INSERT INTO m3 VALUES (1, 'sad'), (2, 'ok'), (3, 'happy')");

    // Re-mint the type over a different member set. The stored `ok` and `happy` are now
    // labels `mood` does not hold; `sad` is still a member, and the only one.
    db.run("DROP TYPE mood");
    db.run("CREATE TYPE mood AS ENUM ('sad')");

    // The member first, then the non-members by their bytes — a total order, and no
    // error: the table is still readable.
    let result = db.run("SELECT id FROM m3 ORDER BY v");
    let ids: Vec<Value> = result.rows.iter().map(|r| r[0].clone()).collect();
    assert_eq!(
        ids,
        vec![Value::Int4(1), Value::Int4(3), Value::Int4(2)],
        "sad (the member) first, then happy/ok by label bytes"
    );

    // ...and the aggregate reads the same order.
    let result = db.run("SELECT min(v), max(v) FROM m3");
    assert_eq!(result.rows[0][0], Value::Text("sad".to_string()));
    assert_eq!(result.rows[0][1], Value::Text("ok".to_string()));
}

/// A `CHECK` added to a table that already holds rows is validated against them at DDL
/// time, by the member order — a *third* `enforce_checks` call site beside the write
/// path's and the `ADD COLUMN` back-fill's, each of which builds its own column scope.
#[test]
fn test_add_check_constraint_validates_existing_rows_by_member_order() {
    let mut db = TestDb::new();
    db.run("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')");
    db.run("CREATE TABLE t (id INT, v mood)");
    db.run("INSERT INTO t VALUES (1, 'happy')");

    // `happy` is declared *after* `ok`, so `v < 'ok'` is false for the stored row and
    // the constraint is refused (`23514`). On the label bytes `happy < ok` holds and
    // the constraint would have been accepted — and then every write re-checked under
    // a rule the existing row breaks.
    let err = db
        .try_run("ALTER TABLE t ADD CONSTRAINT c CHECK (v < 'ok')")
        .expect_err("the stored row violates the new predicate");
    assert_eq!(err.sqlstate(), "23514");

    // The mirror: a predicate the member order satisfies is accepted.
    db.run("ALTER TABLE t ADD CONSTRAINT c2 CHECK (v > 'sad')");
}

/// The `ALTER TABLE ADD COLUMN` back-fill validates its candidate rows against the new
/// column's inline `CHECK` by the member order too — its column scope is the table's
/// columns *plus the one being added*, which is the enum in question.
#[test]
fn test_add_column_backfill_check_uses_member_order() {
    let mut db = TestDb::new();
    db.run("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')");
    db.run("CREATE TABLE t (id INT)");
    db.run("INSERT INTO t VALUES (1)");

    // The back-filled default is `happy`, declared after `ok`, so the inline CHECK
    // refuses the column.
    let err = db
        .try_run("ALTER TABLE t ADD COLUMN v mood DEFAULT 'happy' CHECK (v < 'ok')")
        .expect_err("the back-filled value violates the inline CHECK");
    assert_eq!(err.sqlstate(), "23514");

    // ...and a default the member order satisfies is accepted and back-filled.
    db.run("ALTER TABLE t ADD COLUMN w mood DEFAULT 'sad' CHECK (w < 'ok')");
    let result = db.run("SELECT w FROM t");
    assert_eq!(result.rows[0][0], Value::Text("sad".to_string()));
}
