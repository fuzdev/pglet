//! Engine integration tests: arrays, `unnest`, JSONB, and INSERT … SELECT.

use crate::common;
use common::*;
use pglet_sql::types::{TypeOid, Value};

#[test]
fn test_array_insert_and_select_roundtrip() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE cell (id INT, refs TEXT[])");
    db.run("INSERT INTO cell VALUES (1, ARRAY['a', 'b']), (2, '{c,d}'), (3, NULL)");

    let result = db.run("SELECT refs FROM cell ORDER BY id");
    assert_eq!(result.rows.len(), 3);
    // ARRAY[...] constructor and the '{...}' text literal both land as Value::Array.
    assert_eq!(result.rows[0][0], text_array(&["a", "b"]));
    assert_eq!(result.rows[1][0], text_array(&["c", "d"]));
    assert_eq!(result.rows[2][0], Value::Null);
    // The column reports the text[] OID.
    assert_eq!(result.columns()[0].type_oid, TypeOid::TextArray);
}

#[test]
fn test_array_contains_filter() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE cell (id INT, refs TEXT[])");
    db.run("INSERT INTO cell VALUES (1, ARRAY['x', 'y']), (2, ARRAY['y', 'z']), (3, ARRAY['w'])");

    // `@>` containment, scan-based: rows whose refs contain 'y'.
    let result = db.run("SELECT id FROM cell WHERE refs @> ARRAY['y'] ORDER BY id");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Int4(1));
    assert_eq!(result.rows[1][0], Value::Int4(2));

    // `<@` is the mirror: refs contained by the right-hand array.
    let result = db.run("SELECT id FROM cell WHERE refs <@ ARRAY['w', 'q'] ORDER BY id");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(3));
}

#[test]
fn test_jsonb_build_object_and_containment() {
    use pglet_sql::types::JsonbValue;

    let mut db = TestDb::new();

    // `jsonb_build_object('kind', 'doc')` builds `{"kind": "doc"}` — a text value
    // becomes a JSON string (quoted), not parsed as JSON.
    let result = db.run("SELECT jsonb_build_object('kind', 'doc')");
    assert_eq!(
        result.rows[0][0],
        Value::Jsonb(JsonbValue::Object(vec![(
            "kind".to_string(),
            JsonbValue::String("doc".to_string()),
        )])),
    );
    assert_eq!(result.columns()[0].name, "jsonb_build_object");
    assert_eq!(result.columns()[0].type_oid, TypeOid::Jsonb);

    // `@>` containment: true when the right object's pairs are all present.
    let result =
        db.run("SELECT jsonb_build_object('kind', 'doc') @> jsonb_build_object('kind', 'doc')");
    assert_eq!(result.rows[0][0], Value::Bool(true));

    // false on a value mismatch.
    let result = db.run("SELECT jsonb_build_object('a', '1') @> jsonb_build_object('a', '2')");
    assert_eq!(result.rows[0][0], Value::Bool(false));

    // false when the key is absent from the container.
    let result =
        db.run("SELECT jsonb_build_object('kind', 'doc') @> jsonb_build_object('other', 'x')");
    assert_eq!(result.rows[0][0], Value::Bool(false));

    // multi-key container contains a single-key subset.
    let result = db.run(
        "SELECT jsonb_build_object('kind', 'doc', 'n', 'x') @> jsonb_build_object('kind', 'doc')",
    );
    assert_eq!(result.rows[0][0], Value::Bool(true));

    // `->>` extraction reads a field back as text.
    let result = db.run("SELECT jsonb_build_object('k', 'doc') ->> 'k'");
    assert_eq!(result.rows[0][0], Value::Text("doc".to_string()));

    // A NULL value member is kept as a JSON null.
    let result = db.run("SELECT jsonb_build_object('k', NULL)");
    assert_eq!(
        result.rows[0][0],
        Value::Jsonb(JsonbValue::Object(vec![(
            "k".to_string(),
            JsonbValue::Null,
        )])),
    );

    // Odd argument count is rejected at plan time.
    assert!(
        db.try_run("SELECT jsonb_build_object('only_a_key')")
            .is_err()
    );
}

#[test]
fn test_unnest_projection() {
    let mut db = TestDb::new();

    // `unnest(<array>)` in a no-FROM projection row-expands: one row per array
    // element. The unaliased column is named `unnest`.
    let result = db.run("SELECT unnest(ARRAY['a', 'b', 'c'])");
    assert_eq!(result.columns()[0].name, "unnest");
    assert_eq!(result.rows.len(), 3);
    assert_eq!(result.rows[0][0], Value::Text("a".to_string()));
    assert_eq!(result.rows[1][0], Value::Text("b".to_string()));
    assert_eq!(result.rows[2][0], Value::Text("c".to_string()));

    // A scalar sibling is broadcast across every expanded row.
    let result = db.run("SELECT 'x' AS s, unnest(ARRAY['a', 'b']) AS u");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("x".to_string()));
    assert_eq!(result.rows[0][1], Value::Text("a".to_string()));
    assert_eq!(result.rows[1][0], Value::Text("x".to_string()));
    assert_eq!(result.rows[1][1], Value::Text("b".to_string()));

    // An empty array yields zero rows (matching Postgres).
    let result = db.run("SELECT 'x', unnest('{}'::text[])");
    assert_eq!(result.rows.len(), 0);

    // A set-returning `unnest` outside a projection (here a WHERE predicate) is
    // rejected, not silently collapsed.
    db.run("CREATE TABLE t (id INT)");
    db.run("INSERT INTO t VALUES (1)");
    assert!(
        db.try_run("SELECT id FROM t WHERE id = unnest(ARRAY[1, 2])")
            .is_err()
    );
}

#[test]
fn test_insert_select_unnest() {
    let mut db = TestDb::new();

    // The `query_put_fact_refs` shape: `INSERT … SELECT $src, unnest($targets) ON
    // CONFLICT DO NOTHING`. Here with literal values; the bound form is covered by
    // the differential fixture `spine/fact/put_fact_refs`.
    db.run(
        "CREATE TABLE fact_ref (source_hash TEXT, target_hash TEXT, \
         PRIMARY KEY (source_hash, target_hash))",
    );

    let result = db.run(
        "INSERT INTO fact_ref (source_hash, target_hash) \
         SELECT 'src', unnest(ARRAY['t1', 't2']) \
         ON CONFLICT (source_hash, target_hash) DO NOTHING",
    );
    // Two source rows landed (the `INSERT 0 N` command-tag form).
    assert_eq!(result.command_tag, "INSERT 0 2");

    let result = db.run("SELECT source_hash, target_hash FROM fact_ref ORDER BY target_hash");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("src".to_string()));
    assert_eq!(result.rows[0][1], Value::Text("t1".to_string()));
    assert_eq!(result.rows[1][1], Value::Text("t2".to_string()));

    // Re-inserting an overlapping target set dedups via ON CONFLICT DO NOTHING:
    // only the new `t3` lands; `t2` is skipped.
    let result = db.run(
        "INSERT INTO fact_ref (source_hash, target_hash) \
         SELECT 'src', unnest(ARRAY['t2', 't3']) \
         ON CONFLICT (source_hash, target_hash) DO NOTHING",
    );
    assert_eq!(result.command_tag, "INSERT 0 1");

    let result = db.run("SELECT target_hash FROM fact_ref ORDER BY target_hash");
    assert_eq!(result.rows.len(), 3);
    assert_eq!(result.rows[2][0], Value::Text("t3".to_string()));

    // INSERT … SELECT without ON CONFLICT (and without an explicit column list)
    // maps the source columns positionally onto the leading table columns.
    db.run("CREATE TABLE r (a TEXT, b TEXT)");
    let result = db.run("INSERT INTO r SELECT 's', unnest(ARRAY['u', 'v'])");
    assert_eq!(result.command_tag, "INSERT 0 2");
    let result = db.run("SELECT a, b FROM r ORDER BY b");
    assert_eq!(result.rows[0][0], Value::Text("s".to_string()));
    assert_eq!(result.rows[0][1], Value::Text("u".to_string()));
}

#[test]
fn test_multiple_unnest_projection_rejected() {
    let mut db = TestDb::new();

    // Two set-returning unnests in one no-FROM projection is PG's LCM-zip form,
    // which `plan_select_no_from` does not support — it must fail loud
    // (the `_ => Err("multiple unnest…")` arm), not silently pick one.
    assert!(
        db.try_run("SELECT unnest(ARRAY[1]), unnest(ARRAY[2])")
            .is_err()
    );
}

#[test]
fn test_insert_select_column_count_mismatch_rejected() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (a INT, b INT)");

    // The target column list names two columns but the source query produces
    // one — `insert_select_target_indices` raises the arity mismatch.
    assert!(db.try_run("INSERT INTO t (a, b) SELECT 1").is_err());
}

#[test]
fn test_malformed_insert_select_surfaces_parse_error() {
    let mut db = TestDb::new();

    // A genuinely-malformed `INSERT … SELECT …` that is *not* the ON CONFLICT
    // shape: the parser reads `ON CONFLICT` inline (`at_on_conflict`), so there is
    // no rewrite to mask the parse error.
    assert!(db.try_run("INSERT INTO t (a) SELECT 1 FROM").is_err());
}

#[test]
fn test_array_any() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE cell (id INT)");
    db.run("INSERT INTO cell VALUES (1), (2), (3), (4)");

    // id = ANY(array) — the scan-based membership form the spine uses.
    let result = db.run("SELECT id FROM cell WHERE id = ANY(ARRAY[2, 4]) ORDER BY id");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Int4(2));
    assert_eq!(result.rows[1][0], Value::Int4(4));
}

#[test]
fn test_unnest_single_and_zip() {
    let mut db = TestDb::new();

    // Single-array unnest: one row per element.
    let result = db.run("SELECT x FROM unnest(ARRAY['a', 'b', 'c']) AS t(x) ORDER BY x");
    assert_eq!(result.rows.len(), 3);
    assert_eq!(result.rows[0][0], Value::Text("a".to_string()));
    assert_eq!(result.rows[2][0], Value::Text("c".to_string()));

    // Multi-array unnest zips element-wise (the spine's role/scope_id shape).
    let result = db
        .run("SELECT role, n FROM unnest(ARRAY['ed', 'vw'], ARRAY[1, 2]) AS p(role, n) ORDER BY n");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("ed".to_string()));
    assert_eq!(result.rows[0][1], Value::Int4(1));
    assert_eq!(result.rows[1][0], Value::Text("vw".to_string()));
}

#[test]
fn test_array_cast_text_roundtrip() {
    let mut db = TestDb::new();
    // Casting an array to text yields the `{...}` literal; quoting kicks in for
    // elements with commas/spaces.
    let result = db.run("SELECT ARRAY['a', 'b c', 'd,e']::text AS t");
    assert_eq!(
        result.rows[0][0],
        Value::Text("{a,\"b c\",\"d,e\"}".to_string())
    );
}
