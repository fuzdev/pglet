//! The bound a statement's entry holds its catalog to without walking it
//! (`pglet_sql::depth::CatalogDepth`), and the doors a stored expression comes in by.
//!
//! A statement is refused `54001` at its entry where this thread could not walk the
//! catalog (`crates/pglet/tests/stack_depth.rs` drives that refusal). Measuring the
//! catalog at every statement is a walk over every stored expression, so the entry
//! answers from the deepest expression **any** catalog in this process has taken in
//! (`pglet_sql::depth::stored_depth_seen`) and walks only when that bound refuses.
//!
//! **The bound is only an upper bound while every door raises it.** An expression that
//! reaches a catalog without passing one is one the entry check cannot see, and the
//! walks it stands in front of — a `BEGIN`'s `Clone`, a commit's encoding — abort the
//! process rather than refuse. This is the test that every door does: after each
//! statement, the process bound must still cover the live catalog.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use pglet::Database;
use pglet_sql::depth::{catalog_depth, stored_depth_seen};
use pglet_testing::db::ScratchDir;

/// Run `f` on a server-sized thread, declared, at the server's limit — room for the
/// deepening expressions below.
fn on_large_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(pglet_sql::depth::SERVER_THREAD_STACK)
        .spawn(move || {
            pglet::set_thread_stack(pglet_sql::depth::SERVER_THREAD_STACK);
            pglet::set_thread_max_depth(pglet_sql::depth::SERVER_MAX_DEPTH);
            f()
        })
        .expect("spawn")
        .join()
        .expect("no panic")
}

/// A chain of `n` `+1` terms — an expression `n` levels deep.
fn chain(n: usize) -> String {
    "+1".repeat(n)
}

/// Every route a stored expression reaches a catalog by, each carrying an expression
/// deeper than anything the process has stored before it — so a route that raises the
/// bound for the wrong expression, or not at all, leaves the bound short of the
/// catalog and fails here.
///
/// The assertion is against the process bound as it stands, not a constant: another
/// test in this binary may have stored a deeper expression first, and each statement's
/// own expression is grown past whatever the bound already is.
#[test]
fn every_door_into_a_catalog_raises_the_process_bound() {
    on_large_thread(move || {
        // An in-memory B+tree: these expressions are grown well past one page, and a
        // catalog chunk carries up to 3 KB of the catalog.
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (a INT); CREATE TABLE u (a INT)")
            .expect("tables");
        // Each entry is `{}`-formatted with a chain grown past the current bound.
        let doors: &[&str] = &[
            "CREATE TABLE c1 (a INT CHECK (a{} > 0))",
            "CREATE TABLE c2 (a INT DEFAULT 1{})",
            "CREATE TABLE c3 (a INT, CHECK (a{} > 0))",
            "ALTER TABLE t ADD COLUMN b INT DEFAULT 1{}",
            "ALTER TABLE t ADD COLUMN d INT CHECK (d{} > 0)",
            "ALTER TABLE t ADD CONSTRAINT k CHECK (a{} > 0)",
            "ALTER TABLE t ADD CHECK (a{} > 1)",
            "ALTER TABLE u ALTER COLUMN a SET DEFAULT 1{}",
            "CREATE INDEX i1 ON t ((a{}))",
            "CREATE INDEX i2 ON t (a) WHERE a{} > 0",
            "CREATE UNIQUE INDEX i3 ON u ((a{}))",
            "DO $$ BEGIN CREATE TABLE c4 (a INT CHECK (a{} > 0)); END $$",
            "BEGIN; CREATE TABLE c5 (a INT CHECK (a{} > 0)); COMMIT",
            "BEGIN; CREATE TABLE c6 (a INT CHECK (a{} > 0)); SAVEPOINT s; \
             ALTER TABLE c6 ADD CHECK (a > 0); ROLLBACK TO SAVEPOINT s; COMMIT",
        ];
        for door in doors {
            let deeper = stored_depth_seen() + 20;
            let sql = door.replace("{}", &chain(deeper));
            db.execute(&sql).unwrap_or_else(|e| panic!("{door}: {e}"));
            let live = catalog_depth(db.schema());
            assert!(
                stored_depth_seen() >= live,
                "{door}: the process bound is {} and the catalog is {live} deep",
                stored_depth_seen()
            );
            assert!(live >= deeper, "{door}: the expression is not stored");
        }
        // A rollback restores expressions the bound has already seen.
        db.execute("BEGIN; DROP TABLE c1; DROP TABLE c2; ROLLBACK")
            .expect("rollback");
        assert!(stored_depth_seen() >= catalog_depth(db.schema()));
    });
}

/// A store read back is the other door: its expressions were stored by whatever engine
/// wrote it, so the decode is where they raise the bound. Held to a shallow expression
/// here.
#[test]
fn a_store_read_back_raises_the_process_bound() {
    let dir = ScratchDir::new("catalog_depth_reopen");
    let path = dir.path.clone();
    on_large_thread(move || {
        let mut db = Database::open(&path).expect("open");
        db.execute("CREATE TABLE t (a INT CHECK (a+1+1+1+1+1+1 > 0))")
            .expect("seed");
        let stored = catalog_depth(db.schema());
        drop(db);
        let mut reopened = Database::open(&path).expect("reopen");
        assert_eq!(
            catalog_depth(reopened.schema()),
            stored,
            "the store carries it"
        );
        assert!(stored_depth_seen() >= stored);
        reopened
            .execute("INSERT INTO t (a) VALUES (1)")
            .expect("a write over the reopened catalog");
    });
}

/// A catalog a host mutates behind the record's back is measured again rather than
/// bounded: `Database::schema_mut` hands one out, and nothing about the expressions a
/// caller plants there went through a door.
#[test]
fn a_catalog_handed_out_mutable_is_measured_again() {
    on_large_thread(|| {
        let mut db = Database::with_btree();
        db.execute("CREATE TABLE t (a INT)").expect("table");
        db.execute("SELECT 1").expect("a statement, which measures");
        // The planted expression is deeper than the bound, and only a measurement can
        // see it: the generation does not move, and no door was taken.
        let deep = stored_depth_seen().max(pglet_sql::depth::DEFAULT_MAX_DEPTH) + 50;
        let expr = pglet_sql::parser::parse_expr(&format!("1{} > 0", chain(deep)), db.schema())
            .expect("parse");
        let generation = db.schema().generation;
        db.schema_mut().get_table_mut("t").expect("t").checks.push(
            pglet_sql::catalog::CheckConstraint {
                name: "planted".into(),
                expr,
            },
        );
        assert_eq!(db.schema().generation, generation, "no door, no generation");
        assert!(catalog_depth(db.schema()) > stored_depth_seen());
        // The measurement is what answers, so the statement runs on this large thread
        // and is refused on a small one.
        db.execute("SELECT 1")
            .expect("measured, and this thread walks it");
        let refused = std::thread::Builder::new()
            .stack_size(2 * 1024 * 1024)
            .spawn(move || {
                pglet::set_thread_stack(2 * 1024 * 1024);
                db.execute("SELECT 1").err().map(|e| e.sqlstate())
            })
            .expect("spawn")
            .join()
            .expect("no panic");
        assert_eq!(refused, Some("54001"));
    });
}
