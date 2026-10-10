//! A foreign key 18.6 cannot implement — a column pair with no equality through the
//! referenced key's B-tree operator family — and the halves of it no differential
//! fixture can reach: the catalog a refused statement leaves on the autocommit path
//! (every fixture runs inside `BEGIN … ROLLBACK`, which restores it), and the error
//! fields the harness scores as agreement when pglet omits one.

// A test harness: `.unwrap()`/`panic!` surface a setup failure or an unexpected cell
// shape loudly right here.
#![allow(clippy::unwrap_used, clippy::panic)]

use pglet::Database;
use pglet_core::Severity;

const TEXT_INT_DETAIL: &str = "Key columns \"b\" of the referencing table and \"k\" of the \
                               referenced table are of incompatible types: text and integer.";

/// The two backends a refusal is asked of: the B+tree, whose storage rolls back under a
/// catalog that does not, and the plain map, which rolls back nothing.
fn databases() -> [Database; 2] {
    [Database::with_btree(), Database::in_memory()]
}

/// The refusal carries 18.6's `M` and `D` and nothing else: no `s`/`t`/`c`/`n`, which
/// `ATAddForeignKeyConstraint` does not attach to this error.
#[test]
fn the_refusal_sends_the_message_and_detail_and_no_object_fields() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p (k INT PRIMARY KEY)").unwrap();
    let err = db
        .execute("CREATE TABLE c (b TEXT REFERENCES p)")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "42804");
    assert_eq!(
        err.to_string(),
        "foreign key constraint \"c_b_fkey\" cannot be implemented"
    );
    assert_eq!(err.detail().as_deref(), Some(TEXT_INT_DETAIL));
    let codes: Vec<u8> = err
        .wire_fields(Severity::Error)
        .iter()
        .map(|(code, _)| *code)
        .collect();
    assert_eq!(codes, b"SVCMD".to_vec());
}

/// An enum is named by its identity in the `D`, never by its storage type: `mood`, not
/// `text`.
#[test]
fn the_detail_names_an_enum_by_its_name() {
    let mut db = Database::with_btree();
    db.execute("CREATE TYPE mood AS ENUM ('sad', 'ok')")
        .unwrap();
    db.execute("CREATE TABLE p (k TEXT PRIMARY KEY)").unwrap();
    let err = db
        .execute("CREATE TABLE c (b mood REFERENCES p)")
        .unwrap_err();
    assert_eq!(err.sqlstate(), "42804");
    assert_eq!(
        err.detail().as_deref(),
        Some(
            "Key columns \"b\" of the referencing table and \"k\" of the referenced table \
             are of incompatible types: mood and text."
        )
    );
}

/// A refused `CREATE TABLE` creates nothing it claimed — the table, its `SERIAL`'s
/// sequence, its `UNIQUE` index — inline, table-level and self-referencing alike, and
/// every name is free afterwards.
#[test]
fn a_refused_create_table_leaves_no_phantom() {
    for mut db in databases() {
        db.execute("CREATE TABLE p (k INT PRIMARY KEY)").unwrap();
        for sql in [
            "CREATE TABLE c (a SERIAL, u INT UNIQUE, b TEXT REFERENCES p)",
            "CREATE TABLE c (a SERIAL, u INT UNIQUE, b TEXT, FOREIGN KEY (b) REFERENCES p (k))",
            "CREATE TABLE c (a SERIAL, u INT PRIMARY KEY, b TEXT REFERENCES c)",
        ] {
            let err = db.execute(sql).unwrap_err();
            assert_eq!(err.sqlstate(), "42804", "{sql}");
            assert!(db.schema().get_table("c").is_none(), "{sql}");
            assert!(!db.schema().sequences.contains_key("c_a_seq"), "{sql}");
            assert!(!db.schema().indexes.contains_key("c_u_key"), "{sql}");
            assert!(!db.schema().indexes.contains_key("c_pkey"), "{sql}");
        }
        // The names are free.
        db.execute("CREATE TABLE c (a SERIAL, u INT UNIQUE, b BIGINT REFERENCES p)")
            .unwrap();
        assert_eq!(db.schema().get_table("c").unwrap().foreign_keys.len(), 1);
    }
}

/// A refused `ADD CONSTRAINT` records no key: a row the key would have refused is
/// written afterwards.
#[test]
fn a_refused_add_constraint_records_no_key() {
    for mut db in databases() {
        db.execute("CREATE TABLE p (k INT PRIMARY KEY)").unwrap();
        db.execute("CREATE TABLE c (b TEXT)").unwrap();
        for sql in [
            "ALTER TABLE c ADD CONSTRAINT c_link FOREIGN KEY (b) REFERENCES p",
            "ALTER TABLE c ADD FOREIGN KEY (b) REFERENCES p (k)",
        ] {
            let err = db.execute(sql).unwrap_err();
            assert_eq!(err.sqlstate(), "42804", "{sql}");
            assert!(db.schema().get_table("c").unwrap().foreign_keys.is_empty());
        }
        db.execute("INSERT INTO c VALUES ('no parent')").unwrap();
    }
}

/// A refused `ADD COLUMN` leaves the table as it was on both of its branches — a
/// constant default (no rewrite, the type test ahead of the backfill's `CHECK`; this
/// statement builds no index, and the test's place below one is held by the
/// `fk/add_column_*_build_beats_type_mismatch_loud` fixtures) and a `SERIAL UNIQUE` (a rewrite, the type test above the staged sequence's draws and
/// the column's index build): no column, no sequence, no index, every row readable at
/// its old width.
#[test]
fn a_refused_add_column_leaves_the_table_whole() {
    for mut db in databases() {
        db.execute("CREATE TABLE p (k TEXT PRIMARY KEY)").unwrap();
        db.execute("CREATE TABLE c (z INT)").unwrap();
        db.execute("INSERT INTO c VALUES (1), (2)").unwrap();
        for sql in [
            "ALTER TABLE c ADD COLUMN b INT DEFAULT 1 CHECK (b > 0) REFERENCES p",
            "ALTER TABLE c ADD COLUMN b SERIAL UNIQUE REFERENCES p",
        ] {
            let err = db.execute(sql).unwrap_err();
            assert_eq!(err.sqlstate(), "42804", "{sql}");
            let table = db.schema().get_table("c").unwrap();
            assert_eq!(table.columns.len(), 1, "{sql}");
            assert!(table.foreign_keys.is_empty(), "{sql}");
            assert!(!db.schema().sequences.contains_key("c_b_seq"), "{sql}");
            assert!(!db.schema().indexes.contains_key("c_b_key"), "{sql}");
            let result = db.execute("SELECT * FROM c").unwrap().remove(0);
            assert_eq!(result.rows.len(), 2, "{sql}");
            assert!(result.rows.iter().all(|row| row.len() == 1), "{sql}");
        }
        // The column, its sequence and its index name are free.
        db.execute("ALTER TABLE c ADD COLUMN b SERIAL UNIQUE")
            .unwrap();
    }
}
