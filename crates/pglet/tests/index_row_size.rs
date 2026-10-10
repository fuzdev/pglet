//! Index entries follow 18.6's size limits on every storage backend, through the doors the
//! differential fixtures cannot script, and pglet's own two refusals where it cannot
//! follow them.
//!
//! The fixtures (`tests/fixtures/index_size/`) hold 18.6's boundaries and orders against
//! the oracle. What is here is what a fixture cannot express: a refused multi-row
//! statement leaving no row behind (the harness stops at the error), the same refusals
//! on the file store and on `Database::in_memory()`, a refused `ADD PRIMARY KEY` leaving
//! the catalog as it was, a cascade inside a transaction refused at the statement rather
//! than at `COMMIT`, and the two measured divergences — a key 18.6 compresses under its
//! limit, and a key whose pglet encoding outgrows a B+tree page (`EDGES.md` rows 191 and
//! 192) — which a faithful fixture would hold red.
#![allow(clippy::panic, clippy::expect_used)]

use crate::common;

use common::ScratchDir;
use pglet::Database;
use pglet_sql::types::Value;
use pglet_testing::rng::Xorshift;

/// `len` pseudo-random alphanumerics, which neither engine can compress.
fn letters(seed: u64, len: usize) -> String {
    const ALPHA: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
    let mut rng = Xorshift::new(seed);
    (0..len)
        .map(|_| char::from(ALPHA[rng.below(ALPHA.len() as u64) as usize]))
        .collect()
}

fn count(db: &mut Database, table: &str) -> Value {
    db.execute(&format!("SELECT count(*) FROM {table}"))
        .expect("count")
        .remove(0)
        .rows
        .remove(0)
        .remove(0)
}

/// Every storage backend under test, over the embed facade: the in-memory B+tree, the
/// file store, and the plain in-memory store, which keys through the same `IndexKeying`.
fn hosts(dir: &ScratchDir) -> Vec<(&'static str, Database)> {
    vec![
        ("btree", Database::with_btree()),
        ("file", Database::open(&dir.path).expect("open")),
        ("memory", Database::in_memory()),
    ]
}

/// The B+tree backends roll a failed statement back whole. `Database::in_memory()` is left
/// out: it has no statement rollback at all, so a failed DML's earlier rows stand there
/// whatever failed it (the root `CLAUDE.md`'s `--storage memory` note).
#[test]
fn a_refused_multi_row_insert_leaves_no_row() {
    let dir = ScratchDir::new("index_size_atomic");
    for (host, mut db) in hosts(&dir).into_iter().filter(|(h, _)| *h != "memory") {
        db.execute("CREATE TABLE u (a text UNIQUE)").unwrap();
        let err = db
            .execute(&format!(
                "INSERT INTO u VALUES ('ok'), ('{}')",
                letters(1, 3000)
            ))
            .expect_err("the second row's key is over the limit");
        assert_eq!(err.sqlstate(), "54000", "[{host}] {err}");
        assert_eq!(
            err.to_string(),
            "index row size 3016 exceeds btree version 4 maximum 2704 for index \"u_a_key\"",
            "[{host}]"
        );
        assert_eq!(count(&mut db, "u"), Value::Int8(0), "[{host}]");
        // An UPDATE over several rows is atomic the same way.
        db.execute("INSERT INTO u VALUES ('a'), ('b')").unwrap();
        let long = letters(2, 3000);
        let err = db
            .execute(&format!(
                "UPDATE u SET a = CASE WHEN a = 'b' THEN '{long}' ELSE 'c' END"
            ))
            .expect_err("one row's new key is over the limit");
        assert_eq!(err.sqlstate(), "54000", "[{host}]");
        let rows = db
            .execute("SELECT a FROM u ORDER BY a")
            .unwrap()
            .remove(0)
            .rows;
        assert_eq!(
            rows,
            vec![vec![Value::Text("a".into())], vec![Value::Text("b".into())]],
            "[{host}]"
        );
    }
}

#[test]
fn a_refused_primary_key_leaves_the_catalog_untouched() {
    let dir = ScratchDir::new("index_size_pk");
    for (host, mut db) in hosts(&dir) {
        db.execute("CREATE TABLE t (a text)").unwrap();
        db.execute(&format!("INSERT INTO t VALUES ('{}')", letters(3, 3000)))
            .unwrap();
        let err = db
            .execute("ALTER TABLE t ADD PRIMARY KEY (a)")
            .expect_err("the key is over the limit");
        assert_eq!(err.sqlstate(), "54000", "[{host}] {err}");
        // The column is still nullable and there is no index to enforce anything.
        db.execute("INSERT INTO t VALUES (NULL)")
            .unwrap_or_else(|e| panic!("[{host}] the refused PK left NOT NULL behind: {e}"));
        db.execute("INSERT INTO t VALUES ('x'), ('x')")
            .unwrap_or_else(|e| panic!("[{host}] the refused PK left an index behind: {e}"));
    }
}

/// A foreign key's cascaded write is refused as the action writes it, and inside a
/// transaction at the statement, not at `COMMIT`, on every storage backend.
#[test]
fn a_cascaded_oversized_key_is_refused_at_its_statement() {
    let dir = ScratchDir::new("index_size_cascade");
    for (host, mut db) in hosts(&dir) {
        db.execute("CREATE TABLE p (k text PRIMARY KEY)").unwrap();
        db.execute(
            "CREATE TABLE c (k text REFERENCES p(k) ON UPDATE CASCADE, pad text); \
             CREATE INDEX c_i ON c (k, pad)",
        )
        .unwrap();
        let (k, pad) = (letters(4, 1300), letters(5, 1300));
        db.execute(&format!("INSERT INTO p VALUES ('{k}')"))
            .unwrap();
        db.execute(&format!("INSERT INTO c VALUES ('{k}', '{pad}')"))
            .unwrap();
        db.begin(1).unwrap();
        let err = db
            .execute_as(&format!("UPDATE p SET k = '{}'", letters(6, 1400)), 1)
            .expect_err("the cascaded child key is over the limit");
        assert_eq!(err.sqlstate(), "54000", "[{host}] {err}");
        assert_eq!(
            err.to_string(),
            "index row size 2720 exceeds btree version 4 maximum 2704 for index \"c_i\"",
            "[{host}]"
        );
        db.rollback(1).unwrap();
        db.execute_as(&format!("SELECT count(*) FROM c WHERE k = '{k}'"), 1)
            .unwrap();
    }
}

/// `EDGES.md` row 191: 18.6 compresses a key part over 510 bytes before it measures,
/// so a key that compresses is held there at any length; pglet does not compress and
/// refuses it at its full length — loud, the same `54000` an incompressible key of that
/// length takes on both engines.
#[test]
fn a_compressible_key_is_measured_uncompressed() {
    let dir = ScratchDir::new("index_size_compressible");
    for (host, mut db) in hosts(&dir) {
        db.execute("CREATE TABLE t (a text); CREATE INDEX t_i ON t (a)")
            .unwrap();
        let err = db
            .execute(&format!("INSERT INTO t VALUES ('{}')", "x".repeat(3000)))
            .expect_err("measured at its full length");
        assert_eq!(err.sqlstate(), "54000", "[{host}]");
        assert_eq!(
            err.to_string(),
            "index row size 3016 exceeds btree version 4 maximum 2704 for index \"t_i\"",
            "[{host}]"
        );
    }
}

/// `EDGES.md` row 192: pglet's key encoding escapes each NUL byte of a `bytea` to two,
/// so a NUL-dense key 18.6 holds can outgrow the 4 044 bytes a B+tree page carries as
/// a separator. Refused loud, `54000`, by pglet's own message; the same length with no
/// NULs is held.
#[test]
fn a_key_whose_encoding_outgrows_a_page_is_refused() {
    // 2 692 bytes alternating NUL with a non-zero byte: 18.6's index row is 2 704 bytes
    // (measured accepted), pglet's key 4 054.
    let hex = |bytes: &[u8]| -> String {
        bytes.iter().fold(String::new(), |mut out, b| {
            use std::fmt::Write;
            let _ = write!(out, "{b:02x}");
            out
        })
    };
    let nul_dense = hex(&(0..1346u16)
        .flat_map(|i| [0, 1 + (i % 250) as u8])
        .collect::<Vec<u8>>());
    let plain = hex(&(0..2692u16)
        .map(|i| 1 + (i % 250) as u8)
        .collect::<Vec<u8>>());
    let dir = ScratchDir::new("index_size_encoding");
    for (host, mut db) in hosts(&dir) {
        db.execute("CREATE TABLE t (b bytea); CREATE INDEX t_i ON t (b)")
            .unwrap();
        db.execute(&format!("INSERT INTO t VALUES ('\\x{plain}')"))
            .unwrap_or_else(|e| panic!("[{host}] {e}"));
        let err = db
            .execute(&format!("INSERT INTO t VALUES ('\\x{nul_dense}')"))
            .expect_err("the key encodes past a page");
        assert_eq!(err.sqlstate(), "54000", "[{host}] {err}");
        assert_eq!(
            err.to_string(),
            "index key for index \"t_i\" encodes to 4054 bytes, over the 4044 a pglet B+tree \
             page can hold",
            "[{host}]"
        );
        assert_eq!(count(&mut db, "t"), Value::Int8(1), "[{host}]");
    }
}

/// `EDGES.md` row 191, over a container key: 18.6 compresses a `jsonb` or an array key
/// over 510 bytes before it measures, as it does a `text` one, and each of these fits
/// under its limit there (measured accepted). Here each is measured at its full length
/// and refused, loud, with the size 18.6's layout gives the uncompressed value.
#[test]
fn a_compressible_container_key_is_measured_uncompressed() {
    let numbers = (0..300).map(|n| n.to_string()).collect::<Vec<_>>();
    let cases = [
        (
            "jsonb",
            format!("'\"{}\"'", "x".repeat(3000)),
            "index row size 3024 exceeds btree version 4 maximum 2704 for index \"t_k_key\"",
        ),
        (
            "jsonb",
            format!("'[{}]'", numbers.join(",")),
            "index row size 3616 exceeds btree version 4 maximum 2704 for index \"t_k_key\"",
        ),
        (
            "text[]",
            format!("'{{{}}}'", vec!["\"\""; 669].join(",")),
            "index row size 2712 exceeds btree version 4 maximum 2704 for index \"t_k_key\"",
        ),
    ];
    for (key_type, value, message) in cases {
        let dir = ScratchDir::new("index_size_compressible_container");
        for (host, mut db) in hosts(&dir) {
            db.execute(&format!("CREATE TABLE t (id int, k {key_type} UNIQUE)"))
                .unwrap();
            let err = db
                .execute(&format!("INSERT INTO t (k, id) VALUES ({value}, 1)"))
                .expect_err("measured at its full length");
            assert_eq!(err.sqlstate(), "54000", "[{host}] {key_type}");
            assert_eq!(err.to_string(), message, "[{host}] {key_type}");
            assert_eq!(count(&mut db, "t"), Value::Int8(0), "[{host}]");
        }
    }
}

/// `EDGES.md` row 192, over an array key: an `int4` element's key carries the value's
/// zero bytes, each escaped by the index key's framing — once for a single-column key,
/// once more for a composite one — so an array of small integers outgrows a B+tree
/// page where 18.6's layout, four bytes an element, holds it (measured accepted at
/// each length below). Refused loud, `54000`, by pglet's own message; the table takes
/// the next write.
#[test]
fn an_array_key_whose_encoding_outgrows_a_page_is_refused() {
    let zeros = |n: usize| format!("'{{{}}}'", vec!["0"; n].join(","));
    let dir = ScratchDir::new("index_size_array_encoding");
    for (host, mut db) in hosts(&dir) {
        db.execute("CREATE TABLE t (a int[] UNIQUE)").unwrap();
        db.execute(&format!("INSERT INTO t VALUES ({})", zeros(500)))
            .unwrap_or_else(|e| panic!("[{host}] {e}"));
        let err = db
            .execute(&format!("INSERT INTO t VALUES ({})", zeros(505)))
            .expect_err("the key encodes past a page");
        assert_eq!(
            err.to_string(),
            "index key for index \"t_a_key\" encodes to 4056 bytes, over the 4044 a pglet \
             B+tree page can hold",
            "[{host}]"
        );
        db.execute("INSERT INTO t VALUES ('{1,2,3}')").unwrap();
        assert_eq!(count(&mut db, "t"), Value::Int8(2), "[{host}]");

        db.execute("CREATE TABLE c (n int, a int[], UNIQUE (n, a))")
            .unwrap();
        db.execute(&format!("INSERT INTO c VALUES (1, {})", zeros(300)))
            .unwrap_or_else(|e| panic!("[{host}] {e}"));
        let err = db
            .execute(&format!("INSERT INTO c VALUES (1, {})", zeros(367)))
            .expect_err("the composite key encodes past a page");
        assert_eq!(
            err.to_string(),
            "index key for index \"c_n_a_key\" encodes to 4070 bytes, over the 4044 a pglet \
             B+tree page can hold",
            "[{host}]"
        );
        assert_eq!(count(&mut db, "c"), Value::Int8(1), "[{host}]");
    }
}

/// The GIN refusal on every storage backend, and inside a transaction at its statement.
#[test]
fn a_gin_entry_over_its_limit_is_refused_on_every_backend() {
    let dir = ScratchDir::new("index_size_gin");
    for (host, mut db) in hosts(&dir) {
        db.execute("CREATE TABLE g (a text[]); CREATE INDEX g_i ON g USING gin (a)")
            .unwrap();
        db.execute(&format!(
            "INSERT INTO g VALUES (ARRAY['{}'])",
            letters(7, 2700)
        ))
        .unwrap_or_else(|e| panic!("[{host}] {e}"));
        db.begin(1).unwrap();
        let err = db
            .execute_as(
                &format!("INSERT INTO g VALUES (ARRAY['{}'])", letters(8, 2701)),
                1,
            )
            .expect_err("over the GIN limit");
        assert_eq!(
            err.to_string(),
            "index row size 2720 exceeds maximum 2712 for index \"g_i\"",
            "[{host}]"
        );
        db.rollback(1).unwrap();
        assert_eq!(count(&mut db, "g"), Value::Int8(1), "[{host}]");
    }
}

/// pglet's page bound refuses an index **build** before the DDL writes anything: an
/// `ADD COLUMN … UNIQUE` whose default's key outgrows a page adds no column, and an
/// `ADD PRIMARY KEY` over such a row leaves the column nullable.
#[test]
fn a_build_refused_for_its_page_bound_leaves_the_catalog_untouched() {
    let nul_dense = format!("'\\x{}'", (0..1346).map(|_| "0007").collect::<String>());
    let dir = ScratchDir::new("index_size_page_build");
    for (host, mut db) in hosts(&dir) {
        db.execute("CREATE TABLE t (id int)").unwrap();
        db.execute("INSERT INTO t VALUES (1)").unwrap();
        let err = db
            .execute(&format!(
                "ALTER TABLE t ADD COLUMN b bytea UNIQUE DEFAULT {nul_dense}"
            ))
            .expect_err("the default's key outgrows a page");
        assert_eq!(err.sqlstate(), "54000", "[{host}] {err}");
        db.execute("ALTER TABLE t ADD COLUMN b bytea")
            .unwrap_or_else(|e| panic!("[{host}] the refused ADD COLUMN left `b` behind: {e}"));
        db.execute(&format!("UPDATE t SET b = {nul_dense}"))
            .unwrap();
        let err = db
            .execute("ALTER TABLE t ADD PRIMARY KEY (b)")
            .expect_err("the key outgrows a page");
        assert!(
            err.to_string()
                .contains("over the 4044 a pglet B+tree page can hold"),
            "[{host}] {err}"
        );
        db.execute("INSERT INTO t VALUES (2, NULL)")
            .unwrap_or_else(|e| panic!("[{host}] the refused PK left NOT NULL behind: {e}"));
    }
}

/// Every door that asks the B-tree item limit names the index in `n` and its relation in
/// `t`, and sends 18.6's hint — fields the differential harness scores as agreement when
/// pglet omits them, so an executor that built the refusal with the two names swapped,
/// or a builder that dropped a field, would pass every fixture.
#[test]
fn a_btree_refusal_names_its_index_and_relation_at_every_door() {
    let mut db = Database::with_btree();
    let long = letters(41, 3000);
    db.execute(
        "CREATE TABLE p (k text PRIMARY KEY); INSERT INTO p VALUES ('a'); \
         CREATE TABLE c (k text REFERENCES p ON DELETE SET NULL, pad text); \
         CREATE INDEX c_i ON c (pad, k)",
    )
    .unwrap();
    db.execute(&format!("INSERT INTO c VALUES ('a', '{}')", &long[..2688]))
        .unwrap();
    db.execute("CREATE TABLE t (id int, a text); INSERT INTO t VALUES (1, 'x')")
        .unwrap();
    db.execute(&format!(
        "CREATE TABLE b (a text); INSERT INTO b VALUES ('{long}')"
    ))
    .unwrap();
    db.execute("CREATE INDEX t_i ON t (a)").unwrap();
    let doors = [
        (
            "insert",
            format!("INSERT INTO t VALUES (2, '{long}')"),
            "t",
            "t_i",
        ),
        ("update", format!("UPDATE t SET a = '{long}'"), "t", "t_i"),
        (
            "create index",
            "CREATE INDEX b_i ON b (a)".to_string(),
            "b",
            "b_i",
        ),
        (
            "add primary key",
            "ALTER TABLE b ADD PRIMARY KEY (a)".to_string(),
            "b",
            "b_pkey",
        ),
        ("set null", "DELETE FROM p".to_string(), "c", "c_i"),
    ];
    for (door, sql, table, index) in doors {
        let err = db.execute(&sql).expect_err(door);
        assert_eq!(err.sqlstate(), "54000", "{door}: {err}");
        let location = err.object_location();
        assert_eq!(location.table.as_deref(), Some(table), "{door}");
        assert_eq!(location.constraint.as_deref(), Some(index), "{door}");
        assert_eq!(location.column, None, "{door}");
        assert!(
            err.hint()
                .is_some_and(|h| h.starts_with("Values larger than 1/3 of a buffer page")),
            "{door}"
        );
        assert_eq!(err.detail(), None, "{door}");
    }
}
