//! `UPDATE … FROM`'s `RETURNING` namespace, in the two dimensions the differential
//! harness structurally cannot see.
//!
//! **Duplicate column names.** 18.6's `RETURNING *` over a target and a source that
//! share column names emits every column — `id | v | id | v | k` for `amb_t(id, v)` and
//! `amb_s(id, v, k)` — and the fixture harness keys its recorded rows **by column
//! name**, so the two `id`s and the two `v`s collapse and a build answering three
//! columns compares equal to one answering five. The column *count*, the *order*, and
//! which relation each duplicate came from are all invisible there; here they are the
//! assertion. (The fixtures beside this file therefore use non-colliding names, which
//! is the right choice for them and the reason this file exists.)
//!
//! **Quoted relation names.** The wildcard expansion mints a qualified reference from
//! the *stored* relation name and the executor keys its namespace on the same string,
//! so a name that is not a safe bare identifier round-trips through neither a parser
//! nor a quoter. A fixture would have to spell the name in SQL twice and would pass on
//! any consistent mangling; asserting on the answered rows does not.
//!
//! The backend is `Database::with_btree` — the in-memory CoW B+tree, the default
//! storage — not `Database::in_memory`.

// A test harness: `.unwrap()`/`.expect()`/`panic!` surface a setup failure loudly.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use pglet::Database;
use pglet_core::Value;

/// A database with `amb_t(id, v)` = `(1, 'TARGET')` and `amb_s(id, v, k)` =
/// `(1, 'SOURCE', 7)` — two relations sharing `id` and `v`, differing in arity and in
/// every value, so a wildcard expanded against the wrong relation shows in the shape
/// *and* in the data.
fn colliding_pair() -> Database {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE amb_t (id INT, v TEXT)",
        "CREATE TABLE amb_s (id INT, v TEXT, k INT)",
        "INSERT INTO amb_t VALUES (1, 'TARGET')",
        "INSERT INTO amb_s VALUES (1, 'SOURCE', 7)",
    ] {
        db.execute(sql).unwrap();
    }
    db
}

/// The `(column name, rendered value)` pairs of a single-row result.
fn one_row(db: &mut Database, sql: &str) -> Vec<(String, String)> {
    let mut results = db.execute(sql).unwrap_or_else(|e| panic!("{sql}: {e}"));
    let result = results.remove(0);
    assert_eq!(result.rows.len(), 1, "{sql}");
    let row = &result.rows[0];
    result
        .columns()
        .iter()
        .enumerate()
        .map(|(i, c)| {
            let rendered = match &row[i] {
                Value::Text(t) => t.clone(),
                other => format!("{other:?}"),
            };
            (c.name.clone(), rendered)
        })
        .collect()
}

#[test]
fn bare_wildcard_answers_every_relation_in_from_order() {
    let mut db = colliding_pair();
    // 18.6: `id | v | id | v | k` = `1 | TARGET | 1 | SOURCE | 7`. The **order** is the
    // discriminating half — a build that expanded the source first, or that de-duplicated
    // the repeated names, keeps the same five names and fails here.
    let row = one_row(
        &mut db,
        "UPDATE amb_t SET id = 1 FROM amb_s s WHERE s.id = amb_t.id RETURNING *",
    );
    let names: Vec<&str> = row.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["id", "v", "id", "v", "k"]);
    let values: Vec<&str> = row.iter().map(|(_, v)| v.as_str()).collect();
    assert_eq!(values[1], "TARGET");
    assert_eq!(values[3], "SOURCE");
}

#[test]
fn a_qualified_wildcard_answers_only_its_own_relation() {
    let mut db = colliding_pair();
    // The same statement, twice, differing only in the wildcard's qualifier — which is
    // the pair that separates "the namespace is wider" from "the projection got wider".
    let target = one_row(
        &mut db,
        "UPDATE amb_t SET id = 1 FROM amb_s s WHERE s.id = amb_t.id RETURNING amb_t.*",
    );
    assert_eq!(
        target,
        vec![
            ("id".to_string(), "Int4(1)".to_string()),
            ("v".to_string(), "TARGET".to_string()),
        ]
    );
    let source = one_row(
        &mut db,
        "UPDATE amb_t SET id = 1 FROM amb_s s WHERE s.id = amb_t.id RETURNING s.*",
    );
    assert_eq!(
        source,
        vec![
            ("id".to_string(), "Int4(1)".to_string()),
            ("v".to_string(), "SOURCE".to_string()),
            ("k".to_string(), "Int4(7)".to_string()),
        ]
    );
}

#[test]
fn a_quoted_target_name_expands_and_stays_addressable() {
    let mut db = Database::with_btree();
    for sql in [
        r#"CREATE TABLE "Amb" (id INT, v TEXT)"#,
        "CREATE TABLE amb_s (id INT, v TEXT, k INT)",
        r#"INSERT INTO "Amb" VALUES (1, 'TARGET')"#,
        "INSERT INTO amb_s VALUES (1, 'SOURCE', 7)",
    ] {
        db.execute(sql).unwrap();
    }
    let bare = one_row(
        &mut db,
        r#"UPDATE "Amb" SET id = 1 FROM amb_s s WHERE s.id = "Amb".id RETURNING *"#,
    );
    assert_eq!(
        bare.len(),
        5,
        "the quoted target's columns lead the bare `*`"
    );
    assert_eq!(bare[1], ("v".to_string(), "TARGET".to_string()));
    // And the qualified spelling of the same name reaches the same two columns.
    let qualified = one_row(
        &mut db,
        r#"UPDATE "Amb" SET id = 1 FROM amb_s s WHERE s.id = "Amb".id RETURNING "Amb".*"#,
    );
    assert_eq!(qualified.len(), 2);
    assert_eq!(qualified[1], ("v".to_string(), "TARGET".to_string()));
}

#[test]
fn a_quoted_alias_hides_the_table_name_on_both_sides_of_the_wildcard() {
    let mut db = Database::with_btree();
    for sql in [
        "CREATE TABLE qa_t (id INT, v TEXT)",
        "CREATE TABLE qa_s (id INT, v TEXT, k INT)",
        "INSERT INTO qa_t VALUES (1, 'TARGET')",
        "INSERT INTO qa_s VALUES (1, 'SOURCE', 7)",
    ] {
        db.execute(sql).unwrap();
    }
    let row = one_row(
        &mut db,
        r#"UPDATE qa_t "O x" SET id = 1 FROM qa_s s WHERE s.id = "O x".id RETURNING "O x".*"#,
    );
    assert_eq!(row.len(), 2);
    assert_eq!(row[1], ("v".to_string(), "TARGET".to_string()));
    // An alias hides the table name — 18.6's `42P01`, for the wildcard exactly as for a
    // column. Asserting this beside the accepting spelling is what stops the qualifier
    // from being ignored rather than matched.
    let err = db
        .execute(
            r#"UPDATE qa_t "O x" SET id = 1 FROM qa_s s WHERE s.id = "O x".id RETURNING qa_t.*"#,
        )
        .expect_err("an alias hides the table name");
    assert_eq!(err.sqlstate(), "42P01");
}
