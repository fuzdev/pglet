//! `ALTER TABLE IF EXISTS` — the halves the differential fixtures cannot hold: the
//! `NOTICE` a skip sends (the harness compares no notice), and a skip decided against
//! the catalog a transaction block is building.

use pglet::Database;

/// Over a name no relation holds, every operation — performed, refused, tolerated — is
/// the skip: 18.6's `NOTICE` and the `ALTER TABLE` tag.
#[test]
fn every_operation_skips_with_a_notice_over_a_missing_table() {
    let mut db = Database::with_btree();
    for sql in [
        "ALTER TABLE IF EXISTS nz ADD COLUMN x INT",
        "ALTER TABLE IF EXISTS ONLY nz DROP COLUMN x",
        "ALTER TABLE IF EXISTS nz ALTER COLUMN x SET DEFAULT 1",
        "ALTER TABLE IF EXISTS nz ALTER COLUMN x SET NOT NULL",
        "ALTER TABLE IF EXISTS nz ALTER COLUMN x ADD GENERATED ALWAYS AS IDENTITY",
        "ALTER TABLE IF EXISTS nz ADD CONSTRAINT c CHECK (x > 0)",
        "ALTER TABLE IF EXISTS nz ADD CONSTRAINT c UNIQUE (x)",
        "ALTER TABLE IF EXISTS nz RENAME TO other",
        "ALTER TABLE IF EXISTS ONLY nz DROP CONSTRAINT IF EXISTS nz_pkey",
        "ALTER TABLE IF EXISTS nz ALTER COLUMN x TYPE BIGINT",
        "ALTER TABLE IF EXISTS nz OWNER TO postgres",
    ] {
        let result = db.execute(sql).unwrap().remove(0);
        assert_eq!(result.command_tag, "ALTER TABLE", "{sql}");
        assert_eq!(
            result.notices,
            vec!["relation \"nz\" does not exist, skipping".to_string()],
            "{sql}"
        );
    }
}

/// Over a table that exists the operation runs and sends no skip notice; a refused one is
/// `0A000`, and without `IF EXISTS` a missing table is `42P01` ahead of the refusal.
#[test]
fn a_present_table_runs_the_operation_and_a_missing_one_is_looked_up_first() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (a INT)").unwrap();
    let result = db
        .execute("ALTER TABLE IF EXISTS t ADD COLUMN b INT")
        .unwrap()
        .remove(0);
    assert!(result.notices.is_empty());
    assert_eq!(db.schema().get_table("t").unwrap().columns.len(), 2);

    let err = db
        .execute("ALTER TABLE IF EXISTS t RENAME TO u")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "0A000");
    let err = db.execute("ALTER TABLE nz RENAME TO u").unwrap_err();
    assert_eq!(err.sqlstate(), "42P01");
    let err = db.execute("ALTER TABLE nz DROP CONSTRAINT c").unwrap_err();
    assert_eq!(err.sqlstate(), "42P01");
}

/// Inside a block the skip reads the block's catalog: a table the block created is
/// altered, and one the block dropped is skipped.
#[test]
fn a_block_skips_against_its_own_catalog() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE gone (a INT)").unwrap();
    db.execute("BEGIN").unwrap();
    db.execute("CREATE TABLE made (a INT)").unwrap();
    db.execute("ALTER TABLE IF EXISTS made ADD COLUMN b INT")
        .unwrap();
    db.execute("DROP TABLE gone").unwrap();
    let result = db
        .execute("ALTER TABLE IF EXISTS gone ADD COLUMN b INT")
        .unwrap()
        .remove(0);
    assert_eq!(result.notices.len(), 1);
    db.execute("COMMIT").unwrap();
    assert_eq!(db.schema().get_table("made").unwrap().columns.len(), 2);
}

/// Under a qualifier naming a schema no relation pglet models can be in, every operation
/// is the skip, and the notice names the relation's own name, as 18.6's does — even
/// where `public` holds a table of that name, which is left as it was. A system-schema
/// qualifier is not among them: there 18.6's answer turns on catalog relations pglet
/// does not model, and pglet keeps its `42P01`.
#[test]
fn another_schemas_qualifier_skips_with_the_relations_own_name() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE nz (a INT)").unwrap();
    for sql in [
        "ALTER TABLE IF EXISTS elsewhere.nz ADD COLUMN x INT",
        "ALTER TABLE IF EXISTS ONLY elsewhere.nz DROP CONSTRAINT IF EXISTS nz_pkey",
        "ALTER TABLE IF EXISTS elsewhere.nz RENAME TO other",
        "ALTER TABLE IF EXISTS elsewhere.nz OWNER TO postgres",
        "ALTER TABLE IF EXISTS \"Public\".nz ADD COLUMN x INT",
    ] {
        let result = db.execute(sql).unwrap().remove(0);
        assert_eq!(result.command_tag, "ALTER TABLE", "{sql}");
        assert_eq!(
            result.notices,
            vec!["relation \"nz\" does not exist, skipping".to_string()],
            "{sql}"
        );
    }
    let result = db
        .execute("ALTER TABLE IF EXISTS elsewhere.\"NZ\" ADD COLUMN x INT")
        .unwrap()
        .remove(0);
    assert_eq!(
        result.notices,
        vec!["relation \"NZ\" does not exist, skipping".to_string()]
    );
    assert_eq!(db.schema().get_table("nz").unwrap().columns.len(), 1);

    for sql in [
        "ALTER TABLE IF EXISTS pg_catalog.nz ADD COLUMN x INT",
        "ALTER TABLE IF EXISTS information_schema.nz ADD COLUMN x INT",
        "ALTER TABLE elsewhere.nz ADD COLUMN x INT",
    ] {
        assert_eq!(db.execute(sql).unwrap_err().sqlstate(), "42P01", "{sql}");
    }
}
