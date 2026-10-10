//! Engine integration tests: secondary indexes and PRIMARY KEY / UNIQUE enforcement.

use crate::common;
use common::*;
use pglet_sql::catalog::IndexKey;
use pglet_sql::executor::Storage;
use pglet_sql::types::Value;

#[test]
fn test_create_index_builds_and_maintains_via_executor() {
    // CREATE INDEX through the executor must build the index in storage (the
    // probe matches a scan+filter), back-fill rows that predate it, and stay
    // consistent under INSERT/UPDATE/DELETE — both create orders.
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, k INT)");

    // Insert-then-create: rows exist before the index, so CREATE INDEX back-fills.
    db.run("INSERT INTO t VALUES (1, 10), (2, 20), (3, 10), (4, 30), (5, 20), (6, 10)");
    db.run("CREATE INDEX ix_k ON t(k)");

    let oid = db.schema.get_index("ix_k").expect("index in catalog").oid as u32;

    // Probe == scan+filter for every distinct value (and a miss).
    for probe in [10, 20, 30, 99] {
        let want = Value::Int4(probe);
        let mut got = db
            .storage
            .index_lookup(oid, std::slice::from_ref(&want))
            .expect("lookup");
        got.sort_unstable();
        let expect = db
            .storage
            .scan_filter_ids("t", &[1], std::slice::from_ref(&want));
        assert_eq!(got, expect, "probe {probe}");
    }
    assert_eq!(
        db.storage
            .index_lookup(oid, &[Value::Int4(10)])
            .unwrap()
            .len(),
        3
    );

    // INSERT after create is maintained.
    db.run("INSERT INTO t VALUES (7, 10)");
    let mut after_insert = db.storage.index_lookup(oid, &[Value::Int4(10)]).unwrap();
    after_insert.sort_unstable();
    assert_eq!(
        after_insert,
        db.storage.scan_filter_ids("t", &[1], &[Value::Int4(10)])
    );

    // UPDATE moves a row's key out of one bucket into another.
    db.run("UPDATE t SET k = 20 WHERE id = 1");
    assert!(
        !db.storage
            .index_lookup(oid, &[Value::Int4(10)])
            .unwrap()
            .contains(&0)
    );
    let mut bucket20 = db.storage.index_lookup(oid, &[Value::Int4(20)]).unwrap();
    bucket20.sort_unstable();
    assert_eq!(
        bucket20,
        db.storage.scan_filter_ids("t", &[1], &[Value::Int4(20)])
    );

    // DELETE removes the row from the index.
    db.run("DELETE FROM t WHERE id = 2");
    let mut bucket20_after = db.storage.index_lookup(oid, &[Value::Int4(20)]).unwrap();
    bucket20_after.sort_unstable();
    assert_eq!(
        bucket20_after,
        db.storage.scan_filter_ids("t", &[1], &[Value::Int4(20)])
    );

    // NULL never matches.
    assert!(
        db.storage
            .index_lookup(oid, &[Value::Null])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_create_index_before_insert_back_fills_nothing_then_maintains() {
    // Create-then-insert: the empty index is maintained as rows arrive.
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, k INT)");
    db.run("CREATE INDEX ix_k ON t(k)");
    let oid = db.schema.get_index("ix_k").expect("index in catalog").oid as u32;

    assert!(
        db.storage
            .index_lookup(oid, &[Value::Int4(10)])
            .unwrap()
            .is_empty()
    );

    db.run("INSERT INTO t VALUES (1, 10), (2, 20), (3, 10)");
    let mut got = db.storage.index_lookup(oid, &[Value::Int4(10)]).unwrap();
    got.sort_unstable();
    assert_eq!(
        got,
        db.storage.scan_filter_ids("t", &[1], &[Value::Int4(10)])
    );

    // DROP INDEX removes the def: the probe yields nothing and the catalog entry
    // is gone.
    db.run("DROP INDEX ix_k");
    assert!(db.schema.get_index("ix_k").is_none());
    assert!(
        db.storage
            .index_lookup(oid, &[Value::Int4(10)])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn test_pk_auto_index_naming_and_enforced_flag() {
    // A column-level PRIMARY KEY auto-creates an enforced unique index named
    // `<table>_pkey`; a column-level UNIQUE auto-creates `<table>_<col>_key`.
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT PRIMARY KEY, email TEXT UNIQUE)");

    let pkey = db.schema.get_index("u_pkey").expect("u_pkey exists");
    assert!(pkey.is_unique() && pkey.is_enforced());
    assert_eq!(pkey.key, IndexKey::Columns(vec!["id".to_string()]));

    let ukey = db
        .schema
        .get_index("u_email_key")
        .expect("u_email_key exists");
    assert!(ukey.is_unique() && ukey.is_enforced());
    assert_eq!(ukey.key, IndexKey::Columns(vec!["email".to_string()]));
}

#[test]
fn test_table_level_single_column_pk_unique_enforced() {
    // Table-level single-column PRIMARY KEY / UNIQUE also auto-create enforced
    // indexes (same names as the inline form).
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT, email TEXT, PRIMARY KEY (id), UNIQUE (email))");
    assert!(db.schema.get_index("u_pkey").expect("pkey").is_enforced());
    assert!(
        db.schema
            .get_index("u_email_key")
            .expect("ukey")
            .is_enforced()
    );
}

#[test]
fn test_composite_pk_auto_index_naming_and_enforced() {
    // A composite PRIMARY KEY auto-creates an enforced unique index named
    // `<table>_pkey` carrying both key columns; a composite table-level UNIQUE
    // creates `<table>_<col1>_<col2>_key`.
    let mut db = TestDb::new();
    db.run(
        "CREATE TABLE cf (source_id TEXT, name TEXT, val INT, PRIMARY KEY (source_id, name), \
         UNIQUE (val, name))",
    );
    let pkey = db.schema.get_index("cf_pkey").expect("cf_pkey exists");
    assert!(pkey.is_unique() && pkey.is_enforced());
    assert_eq!(
        pkey.key,
        IndexKey::Columns(vec!["source_id".to_string(), "name".to_string()])
    );

    let ukey = db
        .schema
        .get_index("cf_val_name_key")
        .expect("cf_val_name_key");
    assert!(ukey.is_unique() && ukey.is_enforced());
    assert_eq!(
        ukey.key,
        IndexKey::Columns(vec!["val".to_string(), "name".to_string()])
    );
}

#[test]
fn test_composite_pk_rejects_full_duplicate() {
    // The spine's cell_field(source_id, name) shape: a duplicate (s1,'a') is
    // rejected, but distinct-on-one-column tuples all insert.
    let mut db = TestDb::new();
    db.run("CREATE TABLE cf (source_id TEXT, name TEXT, PRIMARY KEY (source_id, name))");
    db.run("INSERT INTO cf VALUES ('s1', 'a')");
    db.run("INSERT INTO cf VALUES ('s1', 'b')");
    db.run("INSERT INTO cf VALUES ('s2', 'a')");
    assert_eq!(db.storage.scan("cf").unwrap().len(), 3);

    let err = db.try_run("INSERT INTO cf VALUES ('s1', 'a')").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { ref constraint, .. } if constraint == "cf_pkey"),
        "got {err:?}"
    );
    assert_eq!(err.sqlstate(), "23505");
}

#[test]
fn test_composite_pk_differing_in_one_column_allowed() {
    // Two tuples that share the first key column but differ in the second do not
    // conflict (the full tuple is the key, not either column alone).
    let mut db = TestDb::new();
    db.run("CREATE TABLE fr (source_hash TEXT, target_hash TEXT, PRIMARY KEY (source_hash, target_hash))");
    db.run("INSERT INTO fr VALUES ('a', 'b'), ('a', 'c'), ('b', 'b')");
    // The mirror tuple differing only by column order is distinct.
    db.run("INSERT INTO fr VALUES ('b', 'a')");
    assert_eq!(db.storage.scan("fr").unwrap().len(), 4);

    let err = db.try_run("INSERT INTO fr VALUES ('a', 'b')").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );
}

#[test]
fn test_composite_unique_rejects_full_duplicate() {
    // A table-level composite UNIQUE (a, b) rejects a full duplicate.
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (a INT, b TEXT, UNIQUE (a, b))");
    db.run("INSERT INTO u VALUES (1, 'x'), (1, 'y'), (2, 'x')");
    let err = db.try_run("INSERT INTO u VALUES (1, 'x')").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { ref constraint, .. } if constraint == "u_a_b_key"),
        "got {err:?}"
    );
}

#[test]
fn test_composite_unique_allows_null_in_a_key_column() {
    // Composite NULLS DISTINCT: a NULL in any key column means the row is never a
    // uniqueness candidate, so two such rows agreeing on the non-NULL column both
    // insert (matches Postgres).
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (a INT, b TEXT, UNIQUE (a, b))");
    db.run("INSERT INTO u VALUES (1, NULL), (1, NULL), (NULL, 'x'), (NULL, 'x')");
    assert_eq!(db.storage.scan("u").unwrap().len(), 4);
    // But a fully-populated duplicate is still rejected.
    db.run("INSERT INTO u VALUES (2, 'y')");
    let err = db.try_run("INSERT INTO u VALUES (2, 'y')").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );
}

#[test]
fn test_pk_rejects_duplicate_insert() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT PRIMARY KEY, name TEXT)");
    db.run("INSERT INTO u VALUES (1, 'a')");
    let err = db.try_run("INSERT INTO u VALUES (1, 'b')").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { ref constraint, .. } if constraint == "u_pkey"),
        "got {err:?}"
    );
    assert_eq!(err.sqlstate(), "23505");
}

#[test]
fn test_unique_rejects_duplicate_insert() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT, email TEXT UNIQUE)");
    db.run("INSERT INTO u VALUES (1, 'x@y')");
    let err = db.try_run("INSERT INTO u VALUES (2, 'x@y')").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { ref constraint, .. } if constraint == "u_email_key"),
        "got {err:?}"
    );
}

#[test]
fn test_unique_allows_multiple_nulls() {
    // SQL UNIQUE permits multiple NULLs (default NULLS DISTINCT); enforcement skips
    // a NULL key.
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT, email TEXT UNIQUE)");
    db.run("INSERT INTO u VALUES (1, NULL), (2, NULL)");
    let result = db.run("SELECT id FROM u ORDER BY id");
    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_intra_statement_duplicate_rejected() {
    // Two rows in one VALUES list sharing a key: the second probe sees the first
    // (rows are inserted row-by-row before the next probe).
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT PRIMARY KEY)");
    let err = db.try_run("INSERT INTO u VALUES (1), (1)").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );
    // The first row must NOT have landed: the statement fails as a unit at the
    // second row (the executor errors before committing more — here it inserted
    // row 1 then rejected row 2, so storage holds exactly one row).
    let n = db.storage.scan("u").unwrap().len();
    assert_eq!(n, 1);
}

#[test]
fn test_update_to_collide_rejected_self_allowed() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT PRIMARY KEY, name TEXT)");
    db.run("INSERT INTO u VALUES (1, 'a'), (2, 'b')");

    // UPDATE the key onto another row's key → conflict.
    let err = db.try_run("UPDATE u SET id = 2 WHERE id = 1").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );

    // UPDATE a row setting its key to its CURRENT value → self, allowed.
    let ok = db.run("UPDATE u SET id = 1, name = 'a2' WHERE id = 1");
    assert_eq!(ok.command_tag, "UPDATE 1");

    // UPDATE a non-key column → allowed (key unchanged, self excluded).
    let ok2 = db.run("UPDATE u SET name = 'bb' WHERE id = 2");
    assert_eq!(ok2.command_tag, "UPDATE 1");
}

#[test]
fn test_on_conflict_do_update_via_index_probe() {
    // ON CONFLICT (pk) DO UPDATE updates the conflicting row (found via the unique
    // index probe), not insert a duplicate.
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT PRIMARY KEY, name TEXT)");
    db.run("INSERT INTO u VALUES (1, 'a')");
    let r = db.run(
        "INSERT INTO u VALUES (1, 'b') ON CONFLICT (id) DO UPDATE SET name = 'updated' RETURNING name",
    );
    assert_eq!(r.rows, vec![vec![Value::Text("updated".to_string())]]);
    // Still one row.
    assert_eq!(db.storage.scan("u").unwrap().len(), 1);
}

#[test]
fn test_on_conflict_do_nothing_via_index_probe() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT PRIMARY KEY, name TEXT)");
    db.run("INSERT INTO u VALUES (1, 'a')");
    let r = db.run("INSERT INTO u VALUES (1, 'b') ON CONFLICT (id) DO NOTHING");
    assert_eq!(r.command_tag, "INSERT 0 0");
    // The pre-existing row is unchanged.
    let rows = db.run("SELECT name FROM u WHERE id = 1");
    assert_eq!(rows.rows, vec![vec![Value::Text("a".to_string())]]);
}

#[test]
fn test_partial_unique_index_enforced_only_over_predicate() {
    // A PARTIAL unique index (`… WHERE active`) is now ENFORCED, but only over rows
    // the predicate selects: the predicate is re-evaluated per written row. A
    // duplicate among ACTIVE rows is rejected; a duplicate where at least one row is
    // inactive (fails the predicate) inserts cleanly — Postgres semantics.
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT, email TEXT, active BOOL)");
    db.run("CREATE UNIQUE INDEX ux_email_active ON u (email) WHERE active");

    let idx = db
        .schema
        .get_index("ux_email_active")
        .expect("index exists");
    assert!(idx.is_unique(), "still a unique index (for lookup)");
    assert!(
        idx.is_enforced(),
        "partial unique index is enforced over its predicate"
    );
    assert!(
        format!("{:?}", idx.predicate).contains("active"),
        "partial predicate references `active`: {:?}",
        idx.predicate
    );

    // One active + one inactive duplicate email: the inactive row fails the
    // predicate, so it isn't a uniqueness candidate — both insert.
    db.run("INSERT INTO u VALUES (1, 'x@y', true), (2, 'x@y', false)");
    assert_eq!(db.run("SELECT id FROM u ORDER BY id").rows.len(), 2);

    // A second ACTIVE row with the same email collides with row 1 (both in the
    // partial index) and is rejected.
    let err = db
        .try_run("INSERT INTO u VALUES (3, 'x@y', true)")
        .unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );
}

#[test]
fn test_partial_unique_index_predicate_binds_declared_column_types() {
    // A partial unique index whose predicate stringifies a `timestamptz`
    // (`tz::text LIKE '%+00'`) must evaluate that cast with the column's DECLARED
    // type, exactly as a query does. Evaluated untyped, `Value::Timestamp` renders
    // as a plain `timestamp` — no `+00` — so the predicate was FALSE for every row,
    // NO row entered the index, and the index silently enforced nothing: duplicates
    // inserted cleanly. A uniqueness constraint that accepts duplicates is an
    // integrity failure, so this is asserted here rather than as a differential
    // fixture — Postgres rejects `timestamptz::text` in an index predicate as
    // non-IMMUTABLE, so there is no Postgres-side behavior to diff against.
    let mut db = TestDb::new();
    db.run("CREATE TABLE pu (id INT, tz TIMESTAMPTZ)");
    db.run("CREATE UNIQUE INDEX puk ON pu (id) WHERE (tz::text LIKE '%+00')");

    let idx = db.schema.get_index("puk").expect("index exists");
    assert!(idx.is_unique() && idx.is_enforced(), "unique + enforced");

    // The predicate holds for this row (the rendered text carries `+00`), so the
    // row IS a uniqueness candidate…
    db.run("INSERT INTO pu VALUES (1, '2026-07-18 12:30:45+00')");
    // …and a second row with the same key must collide.
    let err = db
        .try_run("INSERT INTO pu VALUES (1, '2026-07-19 01:02:03+00')")
        .unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "duplicate must be rejected, got {err:?}"
    );
    assert_eq!(
        err.sqlstate(),
        "23505",
        "unique_violation SQLSTATE, got {err:?}"
    );
    assert_eq!(
        db.run("SELECT id FROM pu").rows.len(),
        1,
        "no duplicate row"
    );

    // A row the predicate EXCLUDES is not a candidate: `NULL::text LIKE …` is NULL,
    // not true, so it may reuse the key. This pins that the fix keeps the predicate
    // discriminating rather than indexing everything.
    db.run("INSERT INTO pu VALUES (1, NULL)");
    assert_eq!(db.run("SELECT id FROM pu").rows.len(), 2);
}

#[test]
fn test_nulls_not_distinct_index_enforced() {
    // A `NULLS NOT DISTINCT` unique index treats two NULL keys as equal, so the
    // second NULL is rejected (contrast the default NULLS DISTINCT, which allows
    // multiple NULLs).
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT, tag TEXT)");
    db.run("CREATE UNIQUE INDEX ux_tag_nnd ON u (tag) NULLS NOT DISTINCT");
    let idx = db.schema.get_index("ux_tag_nnd").expect("index exists");
    assert!(idx.is_unique() && idx.is_enforced() && idx.nulls_not_distinct);

    db.run("INSERT INTO u VALUES (1, NULL)");
    let err = db.try_run("INSERT INTO u VALUES (2, NULL)").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );

    // Distinct non-null values still insert fine.
    db.run("INSERT INTO u VALUES (3, 'a')");
    db.run("INSERT INTO u VALUES (4, 'b')");
    assert_eq!(
        db.run("SELECT id FROM u WHERE tag IS NOT NULL").rows.len(),
        2
    );
}

#[test]
fn test_plain_create_unique_index_is_enforced() {
    // A plain single-column predicate-free CREATE UNIQUE INDEX is enforced.
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (id INT, email TEXT)");
    db.run("CREATE UNIQUE INDEX ux_email ON u (email)");
    assert!(
        db.schema
            .get_index("ux_email")
            .expect("index")
            .is_enforced()
    );

    db.run("INSERT INTO u VALUES (1, 'x@y')");
    let err = db.try_run("INSERT INTO u VALUES (2, 'x@y')").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );
}

#[test]
fn test_composite_pk_is_enforced() {
    // A composite PRIMARY KEY (>1 column) auto-creates an enforced `<table>_pkey`
    // index over the full tuple, so a full-tuple duplicate is rejected while a
    // tuple differing in one column inserts.
    let mut db = TestDb::new();
    db.run("CREATE TABLE fact_ref (fact INT, target INT, PRIMARY KEY (fact, target))");
    let pkey = db
        .schema
        .get_index("fact_ref_pkey")
        .expect("composite pkey exists");
    assert!(pkey.is_unique() && pkey.is_enforced());
    assert_eq!(
        pkey.key,
        IndexKey::Columns(vec!["fact".to_string(), "target".to_string()])
    );

    db.run("INSERT INTO fact_ref VALUES (1, 2), (1, 3)");
    let err = db
        .try_run("INSERT INTO fact_ref VALUES (1, 2)")
        .unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );
}

#[test]
fn test_composite_create_unique_index_is_enforced() {
    // A plain multi-column CREATE UNIQUE INDEX is now enforced (the `== 1` gate
    // dropped); partial/expression indexes stay non-enforced (separate passes).
    let mut db = TestDb::new();
    db.run("CREATE TABLE u (a INT, b INT)");
    db.run("CREATE UNIQUE INDEX ux_ab ON u (a, b)");
    let ix = db.schema.get_index("ux_ab").expect("index");
    assert!(ix.is_unique() && ix.is_enforced());
    assert_eq!(
        ix.key,
        IndexKey::Columns(vec!["a".to_string(), "b".to_string()])
    );

    db.run("INSERT INTO u VALUES (1, 2), (1, 3)");
    let err = db.try_run("INSERT INTO u VALUES (1, 2)").unwrap_err();
    assert!(
        matches!(err, pglet_sql::Error::UniqueViolation { .. }),
        "got {err:?}"
    );
}

#[test]
fn test_gin_index_ddl_validates_against_column_type() {
    // The GIN access method (`USING gin`) is captured and validated against the key
    // column type: it requires a single array/JSONB column and is never unique. A
    // **non-unique** B-tree index over a single bare array/JSONB column is rejected —
    // the shape the method derivation reads as GIN. A declared `UNIQUE` /
    // `PRIMARY KEY` over the same column, and a `CREATE UNIQUE INDEX`, build a B-tree
    // over it, so read none of this as "such a column is GIN-only".
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT, refs TEXT[], data JSONB, name TEXT)");

    // GIN over an array column and a JSONB column both create.
    db.run("CREATE INDEX gx_refs ON t USING gin(refs)");
    db.run("CREATE INDEX gx_data ON t USING gin(data)");
    // INSERT of these rows must not fail loud: GIN decomposes the value, and both
    // element sets here (`text` elements, JSONB's `Value::Text` tokens) encode.
    db.run("INSERT INTO t VALUES (1, '{a,b}', '{\"kind\":\"note\"}', 'x')");

    // GIN over a scalar column is rejected (no GIN opclass for plain text here).
    assert!(
        db.try_run("CREATE INDEX gx_bad ON t USING gin(name)")
            .is_err()
    );
    // A unique GIN is rejected.
    assert!(
        db.try_run("CREATE UNIQUE INDEX gx_uniq ON t USING gin(refs)")
            .is_err()
    );
    // A B-tree (default) index over an array column is rejected — use USING gin.
    assert!(db.try_run("CREATE INDEX bx_refs ON t (refs)").is_err());
    // A B-tree over the JSONB column is likewise rejected.
    assert!(db.try_run("CREATE INDEX bx_data ON t (data)").is_err());
    // A unique B-tree over either builds, as does a composite one.
    db.run("CREATE UNIQUE INDEX ux_refs ON t (refs)");
    db.run("CREATE UNIQUE INDEX ux_data ON t (data)");
    db.run("CREATE INDEX cx_data ON t (data, id)");
}

#[test]
fn test_gin_array_containment_query_returns_correct_rows() {
    // `refs @> ARRAY[...]` over a GIN index returns the same rows a scan would —
    // the GinScan narrows to candidates and the recheck Filter confirms `@>`.
    let mut db = TestDb::new();
    db.run("CREATE TABLE c (id INT, refs TEXT[])");
    db.run("CREATE INDEX gx ON c USING gin(refs)");
    db.run("INSERT INTO c VALUES (1, '{a,b}'), (2, '{c}'), (3, '{a,c}'), (4, '{}')");

    let ids = |r: pglet_sql::executor::QueryResult| -> Vec<i32> {
        let mut v: Vec<i32> = r
            .rows
            .iter()
            .map(|row| match row[0] {
                Value::Int4(n) => n,
                ref other => panic!("expected int id, got {other:?}"),
            })
            .collect();
        v.sort_unstable();
        v
    };

    // Single element: rows containing 'a'.
    assert_eq!(
        ids(db.run("SELECT id FROM c WHERE refs @> ARRAY['a']")),
        vec![1, 3]
    );
    // Two elements: intersection (must contain BOTH).
    assert_eq!(
        ids(db.run("SELECT id FROM c WHERE refs @> ARRAY['a','c']")),
        vec![3]
    );
    // A miss.
    assert!(ids(db.run("SELECT id FROM c WHERE refs @> ARRAY['z']")).is_empty());
    // Empty array is contained by every row (incl. the empty one).
    assert_eq!(
        ids(db.run("SELECT id FROM c WHERE refs @> ARRAY[]::text[]")),
        vec![1, 2, 3, 4]
    );
}

#[test]
fn test_gin_jsonb_containment_query_returns_correct_rows() {
    // `data @> jsonb_build_object(...)` over a GIN index: the lossy candidate set is
    // narrowed by the recheck Filter to the true containment matches.
    let mut db = TestDb::new();
    db.run("CREATE TABLE c (id INT, data JSONB)");
    db.run("CREATE INDEX gx ON c USING gin(data)");
    db.run(
        "INSERT INTO c VALUES \
         (1, '{\"kind\":\"note\",\"title\":\"x\"}'), \
         (2, '{\"kind\":\"task\"}'), \
         (3, '{\"kind\":\"x\",\"other\":\"note\"}')",
    );

    let r = db.run("SELECT id FROM c WHERE data @> jsonb_build_object('kind', 'note')");
    // Only row 1 truly contains {"kind":"note"} — row 3 has both tokens but not the
    // pairing, so the recheck must drop it.
    assert_eq!(r.rows.len(), 1, "got {:?}", r.rows);
    assert_eq!(r.rows[0][0], Value::Int4(1));
}

#[test]
fn test_gin_correlated_not_exists_orphan_shape() {
    // The spine's orphan-fact shape: a correlated `NOT EXISTS` whose inner `@>`
    // probes `cell.refs` with the OUTER row's hash. The GinScan probe is the
    // correlated `ARRAY[f.hash]`, resolved per outer row.
    let mut db = TestDb::new();
    db.run("CREATE TABLE fact (hash TEXT)");
    db.run("CREATE TABLE cell (id INT, refs TEXT[], deleted BOOL)");
    db.run("CREATE INDEX gx ON cell USING gin(refs) WHERE deleted IS NULL");
    db.run("INSERT INTO fact VALUES ('h1'), ('h2'), ('h3')");
    // h1 referenced by a live cell; h2 only by a deleted cell; h3 unreferenced.
    db.run("INSERT INTO cell VALUES (1, '{h1}', NULL), (2, '{h2}', true)");

    let mut orphans: Vec<String> = db
        .run(
            "SELECT hash FROM fact f WHERE NOT EXISTS ( \
               SELECT 1 FROM cell c WHERE c.refs @> ARRAY[f.hash] AND c.deleted IS NULL \
             )",
        )
        .rows
        .iter()
        .map(|row| match &row[0] {
            Value::Text(s) => s.clone(),
            other => panic!("expected text hash, got {other:?}"),
        })
        .collect();
    orphans.sort();
    // h2 (only a deleted cell references it) and h3 (unreferenced) are orphaned.
    assert_eq!(orphans, vec!["h2".to_string(), "h3".to_string()]);
}

#[test]
fn test_partial_unique_index_predicate_binds_enum_member_order() {
    // The enum sibling of `test_partial_unique_index_predicate_binds_declared_column_types`
    // — the same "a stored predicate must see what a query sees" rule, one type
    // further. An enum value is a `Value::Text`, so the predicate's `m > 'sad'` needs
    // the type's ordered member list or it compares label bytes; `mood`'s declared
    // order is the exact reverse of its lexical one, so the two rules select opposite
    // rows.
    //
    // This runs on `MemStorage`, which keeps its **own** copy of the membership rule
    // (its `MemIndexDef` re-evaluates the predicate), where the fixture corpus runs on
    // the B+tree backend's `IndexKeying` — so the two together are what keep the
    // storage-side halves of this rule in step with the executor's probe.
    let mut db = TestDb::new();
    db.run("CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy')");
    db.run("CREATE TABLE pm (id INT, m mood NOT NULL)");
    db.run("CREATE UNIQUE INDEX pmk ON pm (id) WHERE m > 'sad'");

    // `ok` and `happy` are both declared after `sad`, so both rows are in the index and
    // the duplicate `id` collides. On the label bytes neither row is indexed (`happy`
    // and `ok` both sort below `sad`) and the duplicate lands with no error.
    db.run("INSERT INTO pm VALUES (1, 'ok')");
    let err = db
        .try_run("INSERT INTO pm VALUES (1, 'happy')")
        .expect_err("both rows are in the partial index, so the id duplicates");
    assert_eq!(err.sqlstate(), "23505", "unique_violation, got {err:?}");
    assert_eq!(
        db.run("SELECT id FROM pm").rows.len(),
        1,
        "no duplicate row"
    );

    // The mirror, and the arm that separates "membership by member order" from
    // "index everything": `sad` is the **first** member, so `m > 'sad'` excludes it and
    // the key is reusable. Under the label bytes `sad` is the largest label, both rows
    // would be indexed, and this insert would be refused.
    db.run("CREATE TABLE pm2 (id INT, m mood NOT NULL)");
    db.run("CREATE UNIQUE INDEX pm2k ON pm2 (id) WHERE m > 'sad'");
    db.run("INSERT INTO pm2 VALUES (1, 'sad')");
    db.run("INSERT INTO pm2 VALUES (1, 'sad')");
    assert_eq!(
        db.run("SELECT id FROM pm2").rows.len(),
        2,
        "neither row is in the index, so the duplicate key is fine"
    );
}

#[test]
fn a_range_with_no_lower_bound_reads_a_nulls_not_distinct_index_and_no_table() {
    // The cost half of `index/range_null_key_*`: those fixtures hold which rows such a
    // range answers, and a build that declined the index for it — or fell back to
    // filtering a scan — would answer the same rows. So count the store's whole-table
    // reads: none where the index serves the range, one on the stripped build, whose
    // planner selects no index.
    const INDEXED: usize = if cfg!(feature = "index_acceleration") {
        0
    } else {
        1
    };
    let mut db = TestDb::new();
    db.run("CREATE TABLE r (u INT, n INT, v TEXT)");
    db.run("CREATE UNIQUE INDEX r_u ON r (u) NULLS NOT DISTINCT");
    db.run("CREATE INDEX r_n ON r (n) NULLS NOT DISTINCT");
    db.run("INSERT INTO r VALUES (NULL, NULL, 'null'), (1, NULL, 'one'), (10, 3, 'ten')");
    for (sql, want) in [
        ("SELECT v FROM r WHERE u < 5", vec!["one"]),
        ("SELECT v FROM r WHERE u <= 10 AND v <> 'one'", vec!["ten"]),
        (
            "SELECT v FROM r WHERE 5 > u ORDER BY u DESC LIMIT 1",
            vec!["one"],
        ),
        ("SELECT v FROM r WHERE n < 5", vec!["ten"]),
        ("SELECT v FROM r WHERE n <= 2", vec![]),
    ] {
        let before = db.storage.scan_count();
        let rows = db.run(sql).rows;
        let scans = db.storage.scan_count() - before;
        let got: Vec<&Value> = rows.iter().map(|row| &row[0]).collect();
        let want: Vec<Value> = want.into_iter().map(|s| Value::Text(s.into())).collect();
        assert_eq!(got, want.iter().collect::<Vec<_>>(), "{sql}");
        assert_eq!(scans, INDEXED, "{sql}: whole-table reads");
    }
    // The control: a range over an unindexed column reads the table on both builds.
    let before = db.storage.scan_count();
    db.run("SELECT u FROM r WHERE v < 'p'");
    assert_eq!(db.storage.scan_count() - before, 1, "an unindexed range");
}

/// `random() < 0.5 OR j @> …` draws once a row on both build profiles: over 200 rows none
/// of which contains the probe, a run answers about half of them, never none and never
/// all. Drawn once for the statement — a GIN guard — a run answered 0 or about 100 at
/// random, so thirty runs see a 0 with certainty but for one chance in 2^30.
#[test]
fn a_random_disjunct_beside_a_gin_containment_draws_once_a_row() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE gj (i INT, j JSONB)");
    db.run("CREATE INDEX gj_j ON gj USING gin (j)");
    for i in 0..200 {
        db.run(&format!("INSERT INTO gj VALUES ({i}, '{{}}')"));
    }
    for run in 0..30 {
        let rows = db
            .run(r#"SELECT i FROM gj WHERE random() < 0.5 OR j @> '{"a": 1}'"#)
            .rows
            .len();
        assert!(
            (1..200).contains(&rows),
            "run {run}: {rows} rows — the random() disjunct was drawn once for the statement"
        );
    }
}
