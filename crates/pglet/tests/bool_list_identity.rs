//! A long `AND`/`OR` list's identity survives the portable dump: a partial index's
//! predicate that holds a parenthesized long list in the middle renders it as one
//! nested operand, so the restored predicate is the tree the original was — the same
//! catalog text, and an `ON CONFLICT … WHERE` restating it still matches it.

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use pglet::Database;
use pglet_sql::types::Value;

fn list(lo: usize, n: usize) -> String {
    (lo..lo + n)
        .map(|i| format!("a <> {i}"))
        .collect::<Vec<_>>()
        .join(" AND ")
}

/// 70 operands with a parenthesized 70-operand list at position 35.
fn nested_predicate() -> String {
    let mut items: Vec<String> = (0..70).map(|i| format!("a <> {i}")).collect();
    items.insert(35, format!("({})", list(100, 70)));
    items.join(" AND ")
}

fn text(db: &mut Database, sql: &str) -> String {
    let rows = &db.execute(sql).unwrap()[0].rows;
    let Value::Text(t) = &rows[0][0] else {
        panic!("text")
    };
    t.clone()
}

fn catalog_text(db: &mut Database) -> (String, String) {
    (
        text(
            db,
            "SELECT indexdef FROM pg_indexes WHERE indexname = 'u_nested'",
        ),
        text(
            db,
            "SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'u_check'",
        ),
    )
}

#[test]
fn a_parenthesized_long_list_survives_the_dump() {
    let pred = nested_predicate();
    let mut db = Database::with_btree();
    db.execute(&format!(
        "CREATE TABLE u (a INT, b INT, CONSTRAINT u_check CHECK ({pred}))"
    ))
    .unwrap();
    db.execute(&format!(
        "CREATE UNIQUE INDEX u_nested ON u (a) WHERE {pred}"
    ))
    .unwrap();
    let before = catalog_text(&mut db);
    // The nested list is one operand of the outer one, as 18.6 prints it.
    assert!(
        before.0.contains("(a <> 34) AND ((a <> 100) AND"),
        "{}",
        before.0
    );

    let dump = db.dump_sql().unwrap();
    // The dump is psql input: drop its `COPY … FROM stdin` data blocks (the table is
    // empty) and psql's meta-commands, and the rest reloads through this host.
    let mut in_copy = false;
    let sql: String = dump
        .lines()
        .filter(|l| {
            if in_copy {
                in_copy = *l != "\\.";
                return false;
            }
            in_copy = l.starts_with("COPY ");
            !in_copy && !l.starts_with('\\')
        })
        .collect::<Vec<_>>()
        .join("\n");
    let mut restored = Database::with_btree();
    restored.execute(&sql).expect("the dump reloads");
    assert_eq!(catalog_text(&mut restored), before);

    restored.execute("INSERT INTO u VALUES (500, 1)").unwrap();
    restored
        .execute(&format!(
            "INSERT INTO u VALUES (500, 2) ON CONFLICT (a) WHERE {pred} DO UPDATE SET b = excluded.b"
        ))
        .expect("the restated predicate matches the restored index");
    let rows = &restored.execute("SELECT b FROM u").unwrap()[0].rows;
    assert_eq!(rows, &vec![vec![Value::Int4(2)]]);
}
