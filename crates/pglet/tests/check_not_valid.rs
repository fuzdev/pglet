//! `CHECK … NOT VALID` on a file-backed `Database`: the unvalidated mark is a catalog
//! fact, so it survives a reopen, and a `VALIDATE CONSTRAINT` committed before the reopen
//! is still validated after it, while one taken back with its block is not.

#![expect(
    clippy::unwrap_used,
    reason = "test setup `.unwrap()`s each statement so a failure surfaces where it happens"
)]

use crate::common;

use common::ScratchDir;
use pglet::Database;

/// `convalidated` of each check of `nv`, by name.
fn validated(db: &mut Database) -> Vec<String> {
    let results = db
        .execute(
            "SELECT conname, convalidated FROM pg_constraint WHERE contype = 'c' ORDER BY conname",
        )
        .unwrap();
    results
        .last()
        .unwrap()
        .rows
        .iter()
        .map(|r| format!("{:?}|{:?}", r[0], r[1]))
        .collect()
}

#[test]
fn an_unvalidated_check_stays_unvalidated_across_a_reopen() {
    let dir = ScratchDir::new("check_not_valid");
    {
        let mut db = Database::open(&dir.path).unwrap();
        db.execute("CREATE TABLE nv (id INT PRIMARY KEY, a INT)")
            .unwrap();
        db.execute("INSERT INTO nv VALUES (1, -1), (2, 5)").unwrap();
        db.execute("ALTER TABLE nv ADD CONSTRAINT nv_pos CHECK (a > 0) NOT VALID")
            .unwrap();
        db.execute("ALTER TABLE nv ADD CONSTRAINT nv_small CHECK (a < 100) NOT VALID")
            .unwrap();
        db.execute("ALTER TABLE nv VALIDATE CONSTRAINT nv_small")
            .unwrap();
        db.execute("BEGIN").unwrap();
        db.execute("DELETE FROM nv WHERE a < 0").unwrap();
        db.execute("ALTER TABLE nv VALIDATE CONSTRAINT nv_pos")
            .unwrap();
        db.execute("ROLLBACK").unwrap();
    }
    let mut db = Database::open(&dir.path).unwrap();
    assert_eq!(
        validated(&mut db),
        [
            "Text(\"nv_pos\")|Bool(false)",
            "Text(\"nv_small\")|Bool(true)"
        ]
    );
    assert_eq!(
        db.execute("INSERT INTO nv VALUES (3, -3)")
            .unwrap_err()
            .sqlstate(),
        "23514",
        "an unvalidated check still holds new rows"
    );
    assert_eq!(
        db.execute("ALTER TABLE nv VALIDATE CONSTRAINT nv_pos")
            .unwrap_err()
            .sqlstate(),
        "23514",
        "the rolled-back delete left the violating row"
    );
}
