//! `DROP TABLE` / `DROP INDEX` / `ALTER TABLE … DROP COLUMN` dependency handling over
//! the embed engine.
//!
//! The differential fixtures under `tests/fixtures/ddl/drop_*` validate the *outcomes*
//! byte-for-byte against a live Postgres. What they structurally **cannot** check is
//! state integrity after a refusal: the fixture harness wraps setup + test in
//! `BEGIN … ROLLBACK`, so a `DROP` that errored *after* mutating the catalog would
//! look identical to one that errored before. These tests inspect the catalog
//! afterwards, and cover the round-trip through a reopened store.

// A test harness: `.unwrap()`/`.expect_err()`/`panic!` surface a setup failure or an
// unexpected cell shape loudly right here.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use crate::common;

use common::ScratchDir;
use pglet::Database;
use pglet_sql::types::Value;

/// Run a query and return its single scalar `i64` (a `count(*)` cell).
fn scalar_i64(db: &mut Database, sql: &str) -> i64 {
    let mut results = db.execute(sql).unwrap();
    let result = results.remove(0);
    match result.rows[0][0] {
        Value::Int8(n) => n,
        Value::Int4(n) => i64::from(n),
        ref other => panic!("expected an integer scalar, got {other:?}"),
    }
}

fn sqlstate(db: &mut Database, sql: &str) -> String {
    db.execute(sql)
        .expect_err("statement was expected to fail")
        .sqlstate()
        .to_string()
}

/// A refused `DROP TABLE` must leave the catalog exactly as it was — the parent
/// table, the child, and the FK that blocked it.
#[test]
fn a_blocked_drop_table_leaves_the_catalog_untouched() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE par (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE ch (cid INT PRIMARY KEY, pid INT REFERENCES par(id))")
        .unwrap();
    db.execute("INSERT INTO par VALUES (1)").unwrap();
    db.execute("INSERT INTO ch VALUES (10, 1)").unwrap();

    assert_eq!(sqlstate(&mut db, "DROP TABLE par"), "2BP01");

    // The parent survives with its row, and the FK still enforces.
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM par"), 1);
    assert_eq!(
        sqlstate(&mut db, "INSERT INTO ch VALUES (11, 999)"),
        "23503",
        "the blocking FK must still be enforced after the refusal"
    );
}

/// `CASCADE` sheds only the child's FK constraint: the child table and its rows stay,
/// and the previously-illegal insert is then accepted (Postgres 18.6 behavior).
#[test]
fn drop_table_cascade_sheds_the_constraint_not_the_child() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE par (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE ch (cid INT PRIMARY KEY, pid INT REFERENCES par(id))")
        .unwrap();
    db.execute("INSERT INTO par VALUES (1)").unwrap();
    db.execute("INSERT INTO ch VALUES (10, 1)").unwrap();

    db.execute("DROP TABLE par CASCADE").unwrap();

    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM ch"), 1);
    db.execute("INSERT INTO ch VALUES (11, 999)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM ch"), 2);
    assert_eq!(
        scalar_i64(
            &mut db,
            "SELECT count(*) FROM information_schema.table_constraints \
             WHERE table_name = 'ch' AND constraint_type = 'FOREIGN KEY'",
        ),
        0,
        "the dangling FK entry must be gone from the catalog, not merely inert"
    );
}

/// The index behind a `PRIMARY KEY` / `UNIQUE` cannot be dropped — and the refusal
/// survives a **file-backed reopen**, since the marker rides in the persisted catalog
/// rather than being re-derived from the index's name at read time.
#[test]
fn a_constraint_index_is_undroppable_across_a_reopen() {
    let dir = ScratchDir::new("constraint_index");
    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute("CREATE TABLE t (id INT PRIMARY KEY, k INT UNIQUE, v INT)")
            .unwrap();
        db.execute("CREATE UNIQUE INDEX t_v_ux ON t (v)").unwrap();

        assert_eq!(sqlstate(&mut db, "DROP INDEX t_pkey"), "2BP01");
        assert_eq!(sqlstate(&mut db, "DROP INDEX t_k_key"), "2BP01");
        // A standalone unique index carries no marker and drops freely.
        db.execute("DROP INDEX t_v_ux").unwrap();
    }

    let mut reopened = Database::open(&dir.path).expect("reopen");
    assert_eq!(sqlstate(&mut reopened, "DROP INDEX t_pkey"), "2BP01");
    assert_eq!(sqlstate(&mut reopened, "DROP INDEX t_k_key"), "2BP01");
    // ...and the key is still enforced, which is the point of refusing.
    reopened.execute("INSERT INTO t VALUES (1, 1, 1)").unwrap();
    assert_eq!(
        sqlstate(&mut reopened, "INSERT INTO t VALUES (1, 2, 2)"),
        "23505"
    );
}

/// A declared `CONSTRAINT <name>` names the constraint's index, so the refusal and the
/// catalog both follow the declared name — across a **file-backed reopen**, since the
/// name is persisted rather than re-derived from the table's.
#[test]
fn a_declared_constraint_name_is_the_index_name_across_a_reopen() {
    let dir = ScratchDir::new("declared_constraint_name");
    {
        let mut db = Database::open(&dir.path).expect("open");
        db.execute(
            "CREATE TABLE t (a INT NOT NULL, b INT, CONSTRAINT t_my_pk PRIMARY KEY (a), \
             CONSTRAINT t_my_uq UNIQUE (b))",
        )
        .unwrap();
        assert_eq!(sqlstate(&mut db, "DROP INDEX t_my_pk"), "2BP01");
        // The auto-name was never minted, so it is simply absent.
        assert_eq!(sqlstate(&mut db, "DROP INDEX t_pkey"), "42704");
    }

    let mut reopened = Database::open(&dir.path).expect("reopen");
    assert_eq!(sqlstate(&mut reopened, "DROP INDEX t_my_pk"), "2BP01");
    assert_eq!(sqlstate(&mut reopened, "DROP INDEX t_my_uq"), "2BP01");
    assert_eq!(sqlstate(&mut reopened, "DROP INDEX t_pkey"), "42704");
    assert_eq!(
        scalar_i64(
            &mut reopened,
            "SELECT count(*) FROM information_schema.table_constraints \
             WHERE table_name = 't' AND constraint_name = 't_my_pk'",
        ),
        1
    );
    // ...and the key still enforces under its declared name.
    reopened.execute("INSERT INTO t VALUES (1, 1)").unwrap();
    assert_eq!(
        sqlstate(&mut reopened, "INSERT INTO t VALUES (1, 2)"),
        "23505"
    );
}

/// A foreign key that omits its referenced column list resolves against the parent's
/// **primary key**, found by the catalog's constraint marker — so a parent whose PK
/// carries a declared name still resolves, where a `<table>_pkey` name lookup reported
/// "no unique key" (`42830`).
#[test]
fn an_fk_resolves_a_declared_name_parent_primary_key() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE par (id INT NOT NULL, CONSTRAINT par_my_pk PRIMARY KEY (id))")
        .unwrap();
    db.execute("CREATE TABLE ch (cid INT, pid INT REFERENCES par)")
        .unwrap();
    db.execute("INSERT INTO par VALUES (1)").unwrap();
    db.execute("INSERT INTO ch VALUES (10, 1)").unwrap();
    assert_eq!(
        sqlstate(&mut db, "INSERT INTO ch VALUES (11, 999)"),
        "23503"
    );
}

/// Two constraints on one table cannot share a name (`42710`), which is what keeps the
/// DROP-time resolver's position-based identity from ever having to disambiguate a
/// duplicate through SQL.
#[test]
fn a_duplicate_constraint_name_is_rejected_on_every_path() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p1 (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE p2 (id INT PRIMARY KEY)").unwrap();
    assert_eq!(
        sqlstate(
            &mut db,
            "CREATE TABLE ch (a INT, b INT, CONSTRAINT dup FOREIGN KEY (a) REFERENCES p1(id), \
             CONSTRAINT dup FOREIGN KEY (b) REFERENCES p2(id))",
        ),
        "42710"
    );
    // ...and the rejected CREATE left no phantom table behind.
    assert_eq!(
        scalar_i64(
            &mut db,
            "SELECT count(*) FROM information_schema.tables WHERE table_name = 'ch'",
        ),
        0
    );

    db.execute("CREATE TABLE ch (a INT, b INT)").unwrap();
    db.execute("ALTER TABLE ch ADD CONSTRAINT dup FOREIGN KEY (a) REFERENCES p1(id)")
        .unwrap();
    assert_eq!(
        sqlstate(
            &mut db,
            "ALTER TABLE ch ADD CONSTRAINT dup FOREIGN KEY (b) REFERENCES p2(id)",
        ),
        "42710"
    );
    // The survivor is the first one, still enforcing on `a` alone.
    assert_eq!(
        sqlstate(&mut db, "INSERT INTO ch VALUES (999, 999)"),
        "23503"
    );
    db.execute("INSERT INTO ch VALUES (NULL, 999)").unwrap();
}

/// A refused `DROP COLUMN` leaves the column, its CHECK constraints, and the blocking
/// FK in place.
#[test]
fn a_blocked_drop_column_leaves_the_catalog_untouched() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE par (id INT PRIMARY KEY, code TEXT)")
        .unwrap();
    db.execute("CREATE TABLE ch (cid INT PRIMARY KEY, pid INT REFERENCES par(id))")
        .unwrap();

    assert_eq!(sqlstate(&mut db, "ALTER TABLE par DROP COLUMN id"), "2BP01");

    assert_eq!(
        scalar_i64(
            &mut db,
            "SELECT count(*) FROM information_schema.columns \
             WHERE table_name = 'par' AND column_name = 'id'",
        ),
        1
    );
    assert_eq!(
        scalar_i64(
            &mut db,
            "SELECT count(*) FROM information_schema.table_constraints \
             WHERE table_name = 'ch' AND constraint_type = 'FOREIGN KEY'",
        ),
        1
    );
}

/// Dropping a column sheds the table's own CHECKs over it, so the table stays
/// writable — and re-adding the column does not resurrect them.
#[test]
fn drop_column_sheds_its_checks_and_they_do_not_return() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE t (id INT PRIMARY KEY, a INT, b INT, CHECK (a > 0))")
        .unwrap();
    db.execute("ALTER TABLE t DROP COLUMN a").unwrap();

    db.execute("INSERT INTO t VALUES (1, 5)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM t"), 1);

    db.execute("ALTER TABLE t ADD COLUMN a INT").unwrap();
    db.execute("INSERT INTO t VALUES (2, 6, -1)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM t"), 2);
}

/// The dump of a database whose column drop shed a CHECK and an FK reloads — the
/// state that used to emit `CHECK ((a > 0))` for a column the `CREATE TABLE` no longer
/// declares, and `REFERENCES fkpar (id)` for a table the dump still created but the
/// child no longer had a column for.
#[test]
fn a_post_drop_column_dump_reloads_into_pglet() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE fkpar (id INT PRIMARY KEY)")
        .unwrap();
    db.execute(
        "CREATE TABLE t (id INT PRIMARY KEY, a INT, pid INT REFERENCES fkpar(id), \
         b INT, CHECK (a > 0))",
    )
    .unwrap();
    db.execute("INSERT INTO fkpar VALUES (1)").unwrap();
    db.execute("INSERT INTO t VALUES (1, 5, 1, 9)").unwrap();
    db.execute("ALTER TABLE t DROP COLUMN a").unwrap();
    db.execute("ALTER TABLE t DROP COLUMN pid").unwrap();

    let dump = db.dump_sql().unwrap();
    assert!(
        !dump.contains("CHECK"),
        "the shed CHECK must not survive into the dump:\n{dump}"
    );
    assert!(
        !dump.contains("FOREIGN KEY"),
        "the shed FK must not survive into the dump:\n{dump}"
    );

    // The dump's schema half reloads into a fresh engine. `dump_sql` joins its blocks
    // with a blank line and each `COPY … FROM stdin` block carries its inline data, so
    // splitting on the blank line and skipping the COPY blocks isolates the DDL (the
    // data path is covered end-to-end by `test_dump`).
    let mut restored = Database::with_btree();
    for block in dump.split("\n\n") {
        let sql = block.trim();
        if sql.is_empty() || sql.starts_with("COPY") {
            continue;
        }
        restored
            .execute(sql)
            .unwrap_or_else(|e| panic!("restore failed on `{sql}`: {e}"));
    }
    // The surviving columns are exactly the ones the drops left, and the PRIMARY KEY
    // came back — the consequence `DROP INDEX`'s refusal keeps reachable.
    assert_eq!(
        scalar_i64(
            &mut restored,
            "SELECT count(*) FROM information_schema.columns WHERE table_name = 't'",
        ),
        2
    );
    assert_eq!(
        scalar_i64(
            &mut restored,
            "SELECT count(*) FROM information_schema.table_constraints \
             WHERE table_name = 't' AND constraint_type = 'PRIMARY KEY'",
        ),
        1
    );
    restored.execute("INSERT INTO t VALUES (1, 9)").unwrap();
    assert_eq!(
        sqlstate(&mut restored, "INSERT INTO t VALUES (1, 10)"),
        "23505"
    );
}

/// A column's `DEFAULT` as `information_schema.columns` renders it, or `None`.
fn column_default(db: &mut Database, table: &str, column: &str) -> Option<String> {
    let mut results = db
        .execute(&format!(
            "SELECT column_default FROM information_schema.columns \
             WHERE table_name = '{table}' AND column_name = '{column}'"
        ))
        .unwrap();
    let result = results.remove(0);
    match result.rows.first().map(|row| &row[0]) {
        Some(Value::Text(s)) => Some(s.clone()),
        Some(Value::Null) => None,
        other => panic!("expected a column_default cell, got {other:?}"),
    }
}

/// `DROP TABLE` takes the sequences its columns own — the AUTO edge — so the name is
/// free again.
///
/// **Read the assertion, not the ids.** The observable that used to state this — the
/// re-created table's first id — no longer moves: the leaked `dt_a_seq` made the
/// second `CREATE TABLE`'s namer mint a fresh `dt_a_seq1`, so the first id was `1`
/// under the leak exactly as it is under the fix. What the leak actually cost is
/// **one sequence name per drop/create cycle**, unbounded and invisible to every
/// query that reads a row. So the column's *default* is what this reads.
#[test]
fn drop_table_takes_its_owned_sequences() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE dt (a SERIAL, b INT GENERATED ALWAYS AS IDENTITY, c INT)",
        "INSERT INTO dt (c) VALUES (1)",
        "DROP TABLE dt",
    ] {
        db.execute(sql).unwrap();
    }
    assert!(
        db.schema().sequences.is_empty(),
        "both the SERIAL and the identity sequence go with the table: {:?}",
        db.schema().sequences.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        sqlstate(&mut db, "SELECT nextval('dt_a_seq')"),
        "42P01",
        "the sequence is gone, not merely orphaned"
    );

    db.execute("CREATE TABLE dt (a SERIAL, z INT)").unwrap();
    assert_eq!(
        column_default(&mut db, "dt", "a").as_deref(),
        Some("nextval('dt_a_seq'::regclass)"),
        "a re-created table takes the name back — the leak burned one per cycle"
    );
}

/// `ALTER TABLE … DROP COLUMN` takes the sequence that column owns, and the identity
/// spelling too.
#[test]
fn drop_column_takes_its_owned_sequence() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE dc (a SERIAL, b INT)",
        "INSERT INTO dc (b) VALUES (1)",
        "ALTER TABLE dc DROP COLUMN a",
    ] {
        db.execute(sql).unwrap();
    }
    assert!(
        db.schema().sequences.is_empty(),
        "the sequence went with `a`"
    );
    assert_eq!(sqlstate(&mut db, "SELECT nextval('dc_a_seq')"), "42P01");

    db.execute("ALTER TABLE dc ADD COLUMN c INT GENERATED ALWAYS AS IDENTITY")
        .unwrap();
    db.execute("ALTER TABLE dc DROP COLUMN c").unwrap();
    assert!(
        db.schema().sequences.is_empty(),
        "an identity column's sequence goes the same way"
    );
}

/// A table whose own columns default from its own sequence drops cleanly: every
/// dependent is inside the deletion set.
///
/// The accept side of the refusal below, and the one an over-correction breaks — both
/// spellings, the plain `SERIAL` and a **sibling** column drawing from the same
/// sequence.
#[test]
fn a_tables_own_defaults_never_block_its_drop() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE selfd (a SERIAL, b INT DEFAULT nextval('selfd_a_seq'))")
        .unwrap();
    db.execute("DROP TABLE selfd").unwrap();
    assert!(db.schema().sequences.is_empty());
}

/// A **second** table's default drawing from a sequence this drop would remove is a
/// NORMAL dependent: `2BP01`, and the catalog is untouched.
///
/// This refusal is the correctness prerequisite of the auto-drop above, not an extra:
/// without it the drop leaves the second table's `DEFAULT` naming a sequence that no
/// longer exists, a state neither engine can otherwise reach.
#[test]
fn another_tables_default_blocks_the_drop_and_leaves_the_catalog_untouched() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE owner_t (a SERIAL, b INT)",
        "CREATE TABLE user_t (x INT DEFAULT nextval('owner_t_a_seq'))",
        "INSERT INTO user_t (x) VALUES (DEFAULT)",
    ] {
        db.execute(sql).unwrap();
    }
    for stmt in [
        "DROP TABLE owner_t",
        "DROP TABLE owner_t RESTRICT",
        "DROP TABLE IF EXISTS owner_t",
        "ALTER TABLE owner_t DROP COLUMN a",
        "ALTER TABLE owner_t DROP COLUMN IF EXISTS a",
        "ALTER TABLE owner_t DROP COLUMN a RESTRICT",
    ] {
        assert_eq!(sqlstate(&mut db, stmt), "2BP01", "`{stmt}` must be refused");
    }
    // Nothing moved: the sequence, its owner, and the dependent default all stand.
    assert!(db.schema().sequences.contains_key("owner_t_a_seq"));
    assert_eq!(
        column_default(&mut db, "user_t", "x").as_deref(),
        Some("nextval('owner_t_a_seq'::regclass)")
    );
    db.execute("INSERT INTO user_t (x) VALUES (DEFAULT)")
        .unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT max(x) FROM user_t"), 2);
}

/// `CASCADE` sheds the dependent **default** — never the column, its rows, or the
/// table — on both drop routes.
#[test]
fn cascade_sheds_the_other_tables_default_and_leaves_the_column() {
    for drop in [
        "DROP TABLE owner_t CASCADE",
        "ALTER TABLE owner_t DROP COLUMN a CASCADE",
    ] {
        let mut db = Database::with_btree();
        for sql in [
            "CREATE TABLE owner_t (a SERIAL, b INT)",
            "CREATE TABLE user_t (x INT DEFAULT nextval('owner_t_a_seq'))",
            "INSERT INTO user_t (x) VALUES (DEFAULT)",
        ] {
            db.execute(sql).unwrap();
        }
        db.execute(drop).unwrap();
        assert!(
            !db.schema().sequences.contains_key("owner_t_a_seq"),
            "`{drop}` still takes the sequence"
        );
        assert_eq!(
            column_default(&mut db, "user_t", "x"),
            None,
            "`{drop}` sheds the default"
        );
        assert_eq!(
            scalar_i64(&mut db, "SELECT count(*) FROM user_t"),
            1,
            "`{drop}` keeps the dependent table's rows"
        );
        // The column stays writable, defaulting to NULL now that it has no sequence.
        db.execute("INSERT INTO user_t (x) VALUES (DEFAULT)")
            .unwrap();
        assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM user_t"), 2);
    }
}

/// The deletion set is **column**-scoped on a `DROP COLUMN`, so a *sibling column of
/// the same table* blocks where the whole-table drop succeeds.
///
/// Neither reading of "self-reference does not block" predicts this, which is why it
/// is pinned separately from the accept side above.
#[test]
fn a_sibling_columns_default_blocks_dropping_the_sequences_own_column() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE sib (id SERIAL, b INT DEFAULT nextval('sib_id_seq'))")
        .unwrap();
    assert_eq!(sqlstate(&mut db, "ALTER TABLE sib DROP COLUMN id"), "2BP01");
    db.execute("ALTER TABLE sib DROP COLUMN id CASCADE")
        .unwrap();
    assert_eq!(column_default(&mut db, "sib", "b"), None);
    assert!(db.schema().sequences.is_empty());
}

/// `DROP SEQUENCE` resolves the **kind** on a name that exists, so a wrong-kind target
/// is `42809` with `IF EXISTS` as without — the spelling where "does nothing" would
/// otherwise look like success.
#[test]
fn drop_sequence_wrong_kind_beats_if_exists() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE dst (a INT)").unwrap();
    db.execute("CREATE INDEX dsi ON dst (a)").unwrap();
    for stmt in [
        "DROP SEQUENCE dst",
        "DROP SEQUENCE IF EXISTS dst",
        "DROP SEQUENCE dst CASCADE",
        "DROP SEQUENCE dsi",
        "DROP SEQUENCE IF EXISTS dsi",
    ] {
        assert_eq!(sqlstate(&mut db, stmt), "42809", "`{stmt}`");
    }
    // The relations the wrong-kind statements named are all still there.
    assert!(db.schema().get_table("dst").is_some());
    assert!(db.schema().get_index("dsi").is_some());
    // A name no relation holds at all is the only one that reaches the skip.
    assert_eq!(sqlstate(&mut db, "DROP SEQUENCE nosuch"), "42P01");
    db.execute("DROP SEQUENCE IF EXISTS nosuch").unwrap();
}

/// An identity column's sequence is its INTERNAL implementation: refused **with**
/// `CASCADE`, which is what separates it from the `SERIAL` spelling below.
#[test]
fn drop_sequence_is_refused_by_an_identity_column_even_under_cascade() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE idt (id INT GENERATED ALWAYS AS IDENTITY, b INT)")
        .unwrap();
    for stmt in [
        "DROP SEQUENCE idt_id_seq",
        "DROP SEQUENCE idt_id_seq CASCADE",
        "DROP SEQUENCE IF EXISTS idt_id_seq",
        "DROP SEQUENCE IF EXISTS idt_id_seq CASCADE",
    ] {
        assert_eq!(sqlstate(&mut db, stmt), "2BP01", "`{stmt}`");
    }
    assert!(db.schema().sequences.contains_key("idt_id_seq"));
    db.execute("INSERT INTO idt (b) VALUES (1)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT max(id) FROM idt"), 1);
    // The identity's own edge outranks a second table's default (`findDependentObjects`
    // raises before `reportDependentObjects` ever runs), so `CASCADE` still refuses.
    db.execute("CREATE TABLE idu (x INT DEFAULT nextval('idt_id_seq'))")
        .unwrap();
    assert_eq!(
        sqlstate(&mut db, "DROP SEQUENCE idt_id_seq CASCADE"),
        "2BP01"
    );
    assert!(db.schema().sequences.contains_key("idt_id_seq"));
}

/// A `SERIAL`'s sequence is blocked by the **column default** — a NORMAL edge, so
/// `CASCADE` sheds the default and the column keeps its rows.
#[test]
fn drop_sequence_yields_to_cascade_over_a_column_default() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE sq (id SERIAL, b INT)").unwrap();
    db.execute("INSERT INTO sq (b) VALUES (1)").unwrap();
    assert_eq!(sqlstate(&mut db, "DROP SEQUENCE sq_id_seq"), "2BP01");
    assert_eq!(
        sqlstate(&mut db, "DROP SEQUENCE sq_id_seq RESTRICT"),
        "2BP01"
    );
    assert_eq!(
        sqlstate(&mut db, "DROP SEQUENCE IF EXISTS sq_id_seq"),
        "2BP01",
        "the skip never fires — the name resolves"
    );
    // Nothing moved through three refusals.
    assert!(db.schema().sequences.contains_key("sq_id_seq"));
    assert_eq!(
        column_default(&mut db, "sq", "id").as_deref(),
        Some("nextval('sq_id_seq'::regclass)")
    );

    db.execute("DROP SEQUENCE sq_id_seq CASCADE").unwrap();
    assert!(!db.schema().sequences.contains_key("sq_id_seq"));
    assert_eq!(column_default(&mut db, "sq", "id"), None);
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM sq"), 1);
    // A `SERIAL` column is NOT NULL, so the defaultless insert now fails on that —
    // the column survived the CASCADE with its constraints intact.
    assert_eq!(sqlstate(&mut db, "INSERT INTO sq (b) VALUES (2)"), "23502");
    db.execute("INSERT INTO sq VALUES (9, 2)").unwrap();
}

/// An owned sequence whose column carries **no** default has only the AUTO edge, so it
/// drops cleanly — the accept side of the two refusals above.
#[test]
fn drop_sequence_succeeds_over_an_owner_without_a_default() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE osq (a INT)").unwrap();
    db.execute("CREATE SEQUENCE osq_seq START WITH 5").unwrap();
    db.execute("ALTER SEQUENCE osq_seq OWNED BY osq.a").unwrap();
    db.execute("DROP SEQUENCE osq_seq").unwrap();
    assert!(db.schema().sequences.is_empty());
    assert!(
        db.schema().get_table("osq").is_some(),
        "the owner table is not a dependent of its own sequence"
    );
}

/// A dropped sequence's state does not survive a file-backed reopen, and neither does
/// a `CASCADE`-shed default — both are catalog facts, so the persist path has to carry
/// them.
#[test]
fn a_dropped_sequence_stays_dropped_across_a_reopen() {
    let dir = ScratchDir::new("drop_sequence_reopen");
    {
        let mut db = Database::open(&dir.path).expect("open");
        for sql in [
            "CREATE TABLE rs (id SERIAL, b INT)",
            "CREATE SEQUENCE spare START WITH 7",
            "DROP SEQUENCE rs_id_seq CASCADE",
        ] {
            db.execute(sql).unwrap();
        }
    }
    let mut db = Database::open(&dir.path).expect("reopen");
    assert!(!db.schema().sequences.contains_key("rs_id_seq"));
    assert!(db.schema().sequences.contains_key("spare"));
    assert_eq!(column_default(&mut db, "rs", "id"), None);
    assert_eq!(sqlstate(&mut db, "SELECT nextval('rs_id_seq')"), "42P01");
}

// ---------------------------------------------------------------------------
// Multi-name `DROP TABLE a, b` — the three-phase split.
//
// These are unit tests and not fixtures for the reason the module header gives one
// scale up: the differential harness wraps every fixture in `BEGIN … ROLLBACK`, which
// restores the catalog snapshot, so a `DROP` that removed one table and *then* raised
// looks exactly like one that raised before touching anything. On the autocommit path
// it does not: `run_planned_env` rolls back **storage** and leaves `schema` mutated in
// place, and the next successful write serializes the whole catalog — so the first
// target is durably gone. `Database::with_btree` is the backend that shows it; the
// plain in-memory storage double never rolls back at all.
// ---------------------------------------------------------------------------

/// A phase-1 failure on the **second** name must leave the **first** table entirely in
/// place — in the catalog, readable, and its name still taken.
///
/// The three assertions are one class in three spellings, and the third is the one a
/// catalog-only check misses: a table `DROP`ped from `schema` but rolled back in
/// storage reads `42P01` and re-creates as `42P07` forever.
#[test]
fn a_failed_multi_drop_leaves_every_table_in_place() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE only_c (id INT)").unwrap();
    db.execute("INSERT INTO only_c VALUES (1)").unwrap();

    assert_eq!(sqlstate(&mut db, "DROP TABLE only_c, nosuch2"), "42P01");

    assert!(db.schema().get_table("only_c").is_some());
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM only_c"), 1);
    assert_eq!(
        sqlstate(&mut db, "CREATE TABLE only_c (id INT)"),
        "42P07",
        "the surviving name must still be taken"
    );
    // And the statement is repeatable — a half-applied first attempt would make the
    // retry's own first name missing.
    assert_eq!(sqlstate(&mut db, "DROP TABLE only_c, nosuch2"), "42P01");
}

/// The same, with the second name a relation of the **wrong kind** (`42809`) rather
/// than a missing one — a different phase-1 arm reaching the same requirement.
#[test]
fn a_wrong_kind_multi_drop_leaves_every_table_in_place() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE wk_t (id INT)").unwrap();
    db.execute("INSERT INTO wk_t VALUES (7)").unwrap();
    db.execute("CREATE SEQUENCE wk_s").unwrap();

    assert_eq!(sqlstate(&mut db, "DROP TABLE wk_t, wk_s"), "42809");
    // `IF EXISTS` does not soften a wrong kind — and must not soften the atomicity.
    assert_eq!(
        sqlstate(&mut db, "DROP TABLE IF EXISTS wk_t, wk_s"),
        "42809"
    );

    assert!(db.schema().get_table("wk_t").is_some());
    assert!(db.schema().sequences.contains_key("wk_s"));
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM wk_t"), 1);
    assert_eq!(sqlstate(&mut db, "CREATE TABLE wk_t (id INT)"), "42P07");
}

/// A **phase-2** refusal — the second target is FK-referenced from outside the list —
/// must also leave the first target in place. Phase 2 runs over every survivor before
/// phase 3 mutates anything, so no `2BP01` can arrive after a removal.
#[test]
fn a_multi_drop_blocked_at_phase_two_leaves_every_table_in_place() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE p2a (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE p2b (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE p2out (x INT REFERENCES p2b(id))")
        .unwrap();
    db.execute("INSERT INTO p2a VALUES (1)").unwrap();

    assert_eq!(sqlstate(&mut db, "DROP TABLE p2a, p2b"), "2BP01");

    assert!(db.schema().get_table("p2a").is_some());
    assert!(db.schema().get_table("p2b").is_some());
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM p2a"), 1);
    assert_eq!(sqlstate(&mut db, "CREATE TABLE p2a (id INT)"), "42P07");
    // The blocking FK still enforces — nothing about the refusal touched it.
    assert_eq!(sqlstate(&mut db, "INSERT INTO p2out VALUES (999)"), "23503");
}

/// The **position-drift** guard: two targets referenced by two constraints on one
/// outside table, shed in one merged catalog removal.
///
/// Per-target removal computes both positions against the pre-mutation catalog, so
/// dropping `ca` slides `cext`'s second constraint from position 1 to 0 and the second
/// call's `if i < child.foreign_keys.len()` **silently no-ops** — leaving
/// `cext_y_fkey` pointing at a table that no longer exists. That state is invisible to
/// any single-target test, and the `INSERT` is what makes it observable rather than
/// merely untidy: `enforce_foreign_keys` treats a missing parent tuple as a violation,
/// so every later write to `cext` would be `23503` naming a vanished table.
#[test]
fn a_multi_drop_cascade_sheds_every_positions_fk() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE ca (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE cb (id INT PRIMARY KEY)").unwrap();
    db.execute("CREATE TABLE cext (x INT REFERENCES ca, y INT REFERENCES cb)")
        .unwrap();

    db.execute("DROP TABLE ca, cb CASCADE").unwrap();

    assert_eq!(
        db.schema()
            .get_table("cext")
            .expect("the child table survives a CASCADE")
            .foreign_keys
            .len(),
        0,
        "both constraints must be shed, not just the first position's"
    );
    assert_eq!(
        scalar_i64(
            &mut db,
            "SELECT count(*) FROM information_schema.table_constraints \
             WHERE table_name = 'cext' AND constraint_type = 'FOREIGN KEY'",
        ),
        0
    );
    db.execute("INSERT INTO cext VALUES (99, 99)").unwrap();
    assert_eq!(scalar_i64(&mut db, "SELECT count(*) FROM cext"), 1);
}

/// `IF EXISTS` notices once per **written** name, not once per distinct missing
/// relation: the skip is a per-list-element event (`DropErrorMsgNonExistent` runs
/// inside `RemoveRelations`' loop), while the deletion set is a set.
///
/// Not a fixture: `NOTICE`s are not part of the compared result shape.
#[test]
fn a_multi_drop_if_exists_notices_once_per_written_name() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE q1 (id INT)").unwrap();

    let mut results = db.execute("DROP TABLE IF EXISTS q1, nz, q1").unwrap();
    let result = results.remove(0);
    assert_eq!(
        result.notices,
        vec!["table \"nz\" does not exist, skipping".to_string()],
    );
    assert!(db.schema().get_table("q1").is_none());

    // Both spellings of the same missing name notice twice — one per element.
    let mut results = db.execute("DROP TABLE IF EXISTS nz, nz").unwrap();
    assert_eq!(results.remove(0).notices.len(), 2);
}

/// A duplicated **surviving** name is one target, not two: the second occurrence must
/// not re-resolve against a catalog the first has already changed.
#[test]
fn a_duplicated_drop_target_is_one_deletion() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE dup1 (id INT)").unwrap();
    db.execute("DROP TABLE dup1, dup1").unwrap();
    assert!(db.schema().get_table("dup1").is_none());
}

/// A mutual FK cycle is droppable in one statement and in neither single-target order
/// — the whole reason phase 2 runs over the union rather than per name.
#[test]
fn a_mutual_fk_cycle_drops_as_one_statement() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE f1 (id INT PRIMARY KEY, o INT)")
        .unwrap();
    db.execute("CREATE TABLE f2 (id INT PRIMARY KEY, o INT)")
        .unwrap();
    db.execute("ALTER TABLE f1 ADD CONSTRAINT f1_o_fkey FOREIGN KEY (o) REFERENCES f2(id)")
        .unwrap();
    db.execute("ALTER TABLE f2 ADD CONSTRAINT f2_o_fkey FOREIGN KEY (o) REFERENCES f1(id)")
        .unwrap();

    assert_eq!(sqlstate(&mut db, "DROP TABLE f1"), "2BP01");
    assert_eq!(sqlstate(&mut db, "DROP TABLE f2"), "2BP01");
    db.execute("DROP TABLE f1, f2").unwrap();
    assert!(db.schema().get_table("f1").is_none());
    assert!(db.schema().get_table("f2").is_none());
}

/// A multi-name `DROP SEQUENCE` refused in phase 2 leaves every named sequence — and
/// its counter — where they were.
#[test]
fn a_failed_multi_drop_sequence_leaves_every_sequence_in_place() {
    let mut db = Database::with_btree();
    db.execute("CREATE SEQUENCE ms1 START WITH 5").unwrap();
    db.execute("CREATE TABLE msi (id INT GENERATED ALWAYS AS IDENTITY, b INT)")
        .unwrap();

    // The identity column's sequence is an INTERNAL dependent: `2BP01` with CASCADE too.
    assert_eq!(
        sqlstate(&mut db, "DROP SEQUENCE ms1, msi_id_seq CASCADE"),
        "2BP01"
    );
    // ...and a phase-1 failure on a later name outranks it.
    assert_eq!(
        sqlstate(&mut db, "DROP SEQUENCE msi_id_seq, nosuch_seq"),
        "42P01"
    );

    assert!(db.schema().sequences.contains_key("ms1"));
    assert!(db.schema().sequences.contains_key("msi_id_seq"));
    assert_eq!(scalar_i64(&mut db, "SELECT nextval('ms1')"), 5);
}

/// A multi-name `DROP INDEX` refused on the constraint-owned index leaves the plain
/// one — the refusal is phase 2, and phase 3 has not run.
#[test]
fn a_failed_multi_drop_index_leaves_every_index_in_place() {
    let mut db = Database::with_btree();
    db.execute("CREATE TABLE mi (a INT PRIMARY KEY, b INT)")
        .unwrap();
    db.execute("CREATE INDEX mi_b ON mi (b)").unwrap();

    assert_eq!(sqlstate(&mut db, "DROP INDEX mi_b, mi_pkey"), "2BP01");
    assert_eq!(sqlstate(&mut db, "DROP INDEX mi_pkey, mi_b"), "2BP01");
    // A later name's phase-1 failure outranks the phase-2 refusal.
    assert_eq!(sqlstate(&mut db, "DROP INDEX mi_pkey, nosuch_ix"), "42704");

    assert!(db.schema().get_index("mi_b").is_some());
    assert!(db.schema().get_index("mi_pkey").is_some());
    // The key is still enforced.
    db.execute("INSERT INTO mi VALUES (1, 1)").unwrap();
    assert_eq!(sqlstate(&mut db, "INSERT INTO mi VALUES (1, 2)"), "23505");

    db.execute("DROP INDEX mi_b").unwrap();
    assert!(db.schema().get_index("mi_b").is_none());
}
