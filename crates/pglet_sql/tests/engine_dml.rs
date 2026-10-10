//! Engine integration tests: DML — INSERT/SELECT/UPDATE/DELETE, NULL handling,
//! aggregates, joins, expressions, type coercion, and error handling.

use crate::common;
use common::*;
use pglet_sql::types::Value;

#[test]
fn test_insert_single_row() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE users (id INT, name TEXT)");

    let result = db.run("INSERT INTO users VALUES (1, 'Alice')");
    assert_eq!(result.command_tag, "INSERT 0 1");
}

#[test]
fn test_insert_multi_row() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE users (id INT, name TEXT)");

    let result = db.run("INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')");
    assert_eq!(result.command_tag, "INSERT 0 3");
}

#[test]
fn test_insert_returning() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE users (id INT, name TEXT)");

    let result = db.run("INSERT INTO users VALUES (1, 'Alice') RETURNING id, name");
    assert_eq!(result.columns().len(), 2);
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(1));
    assert_eq!(result.rows[0][1], Value::Text("Alice".to_string()));
}

#[test]
fn test_select_star() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("SELECT * FROM users ORDER BY id");
    assert_eq!(result.columns().len(), 3);
    assert_eq!(result.rows.len(), 3);
    assert_eq!(result.rows[0][1], Value::Text("Alice".to_string()));
    assert_eq!(result.rows[1][1], Value::Text("Bob".to_string()));
    assert_eq!(result.rows[2][1], Value::Text("Carol".to_string()));
}

#[test]
fn test_select_where() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("SELECT name FROM users WHERE id = 2");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Text("Bob".to_string()));
}

#[test]
fn test_select_order_by() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("SELECT name FROM users ORDER BY name DESC");
    assert_eq!(result.rows.len(), 3);
    assert_eq!(result.rows[0][0], Value::Text("Carol".to_string()));
    assert_eq!(result.rows[1][0], Value::Text("Bob".to_string()));
    assert_eq!(result.rows[2][0], Value::Text("Alice".to_string()));
}

#[test]
fn test_select_limit_offset() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("SELECT name FROM users ORDER BY id LIMIT 2");
    assert_eq!(result.rows.len(), 2);

    let result = db.run("SELECT name FROM users ORDER BY id OFFSET 1");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("Bob".to_string()));
}

#[test]
fn test_select_boolean_filter() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("SELECT name FROM users WHERE active = true ORDER BY name");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("Alice".to_string()));
    assert_eq!(result.rows[1][0], Value::Text("Carol".to_string()));
}

#[test]
fn test_null_comparison() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE test (id INT, value INT)");
    db.run("INSERT INTO test VALUES (1, 10), (2, NULL), (3, 30)");

    // NULL = NULL should return no rows (SQL three-valued logic)
    let result = db.run("SELECT id FROM test WHERE value = NULL");
    assert_eq!(result.rows.len(), 0);

    // IS NULL
    let result = db.run("SELECT id FROM test WHERE value IS NULL");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(2));

    // IS NOT NULL
    let result = db.run("SELECT id FROM test WHERE value IS NOT NULL ORDER BY id");
    assert_eq!(result.rows.len(), 2);
}

#[test]
fn test_is_distinct_from() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE idf (id INT, a INT, b INT)");
    db.run(
        "INSERT INTO idf (id, a, b) VALUES \
         (1, NULL, NULL), (2, NULL, 5), (3, 5, NULL), (4, 5, 5), (5, 5, 7)",
    );

    // Both operators ALWAYS return a concrete boolean — never NULL — covering
    // all four NULL/non-NULL combinations.
    let result =
        db.run("SELECT a IS DISTINCT FROM b, a IS NOT DISTINCT FROM b FROM idf ORDER BY id");
    // (id, IS DISTINCT FROM, IS NOT DISTINCT FROM)
    let expected = [
        (Value::Bool(false), Value::Bool(true)), // both NULL → not distinct
        (Value::Bool(true), Value::Bool(false)), // left NULL → distinct
        (Value::Bool(true), Value::Bool(false)), // right NULL → distinct
        (Value::Bool(false), Value::Bool(true)), // equal → not distinct
        (Value::Bool(true), Value::Bool(false)), // unequal → distinct
    ];
    assert_eq!(result.rows.len(), expected.len());
    for (row, (distinct, not_distinct)) in result.rows.iter().zip(expected.iter()) {
        assert_eq!(row[0], *distinct);
        assert_eq!(row[1], *not_distinct);
    }

    // Filter usage: `IS DISTINCT FROM` keeps the NULL rows (a NULL value is
    // distinct from a literal), which `<>` would drop. Rows 1 and 2 have a=NULL;
    // rows 3/4/5 have a=5, so they are not distinct from 5 and are excluded.
    let result = db.run("SELECT id FROM idf WHERE a IS DISTINCT FROM 5 ORDER BY id");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Int4(1)); // a IS NULL → distinct from 5
    assert_eq!(result.rows[1][0], Value::Int4(2)); // a IS NULL → distinct from 5
}

#[test]
fn test_count_star() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("SELECT COUNT(*) FROM users");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int8(3));
}

#[test]
fn test_count_with_null() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE test (id INT, value INT)");
    db.run("INSERT INTO test VALUES (1, 10), (2, NULL), (3, 30)");

    // COUNT(*) counts all rows
    let result = db.run("SELECT COUNT(*) FROM test");
    assert_eq!(result.rows[0][0], Value::Int8(3));

    // COUNT(column) excludes NULLs
    let result = db.run("SELECT COUNT(value) FROM test");
    assert_eq!(result.rows[0][0], Value::Int8(2));
}

#[test]
fn test_sum_avg_min_max() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE numbers (n INT)");
    db.run("INSERT INTO numbers VALUES (10), (20), (30)");

    let result = db.run("SELECT SUM(n) FROM numbers");
    assert_eq!(result.rows[0][0], Value::Int8(60));

    let result = db.run("SELECT MIN(n), MAX(n) FROM numbers");
    assert_eq!(result.rows[0][0], Value::Int4(10));
    assert_eq!(result.rows[0][1], Value::Int4(30));
}

#[test]
fn test_group_by() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE sales (category TEXT, amount INT)");
    db.run("INSERT INTO sales VALUES ('A', 10), ('B', 20), ('A', 30), ('B', 40)");

    let result =
        db.run("SELECT category, SUM(amount) FROM sales GROUP BY category ORDER BY category");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("A".to_string()));
    assert_eq!(result.rows[0][1], Value::Int8(40));
    assert_eq!(result.rows[1][0], Value::Text("B".to_string()));
    assert_eq!(result.rows[1][1], Value::Int8(60));
}

#[test]
fn test_having() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE sales (category TEXT, amount INT)");
    db.run("INSERT INTO sales VALUES ('A', 10), ('B', 20), ('A', 30), ('B', 40)");

    let result =
        db.run("SELECT category, SUM(amount) FROM sales GROUP BY category HAVING SUM(amount) > 50");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Text("B".to_string()));
    assert_eq!(result.rows[0][1], Value::Int8(60));
}

#[test]
fn test_update_with_where() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("UPDATE users SET active = false WHERE id = 1");
    assert_eq!(result.command_tag, "UPDATE 1");

    let result = db.run("SELECT active FROM users WHERE id = 1");
    assert_eq!(result.rows[0][0], Value::Bool(false));
}

#[test]
fn test_update_returning() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("UPDATE users SET name = 'Updated' WHERE id = 1 RETURNING id, name");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(1));
    assert_eq!(result.rows[0][1], Value::Text("Updated".to_string()));
}

#[test]
fn test_delete_with_where() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("DELETE FROM users WHERE id = 2");
    assert_eq!(result.command_tag, "DELETE 1");

    let result = db.run("SELECT COUNT(*) FROM users");
    assert_eq!(result.rows[0][0], Value::Int8(2));
}

#[test]
fn test_delete_returning() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("DELETE FROM users WHERE id = 1 RETURNING name");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Text("Alice".to_string()));
}

#[test]
fn test_inner_join() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE orders (id INT, user_id INT)");
    db.run("CREATE TABLE users (id INT, name TEXT)");
    db.run("INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob')");
    db.run("INSERT INTO orders VALUES (100, 1), (101, 1), (102, 2)");

    let result = db.run("SELECT users.name, orders.id FROM users INNER JOIN orders ON users.id = orders.user_id ORDER BY orders.id");
    assert_eq!(result.rows.len(), 3);
    assert_eq!(result.rows[0][0], Value::Text("Alice".to_string()));
    assert_eq!(result.rows[0][1], Value::Int4(100));
}

#[test]
fn test_left_join() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE users (id INT, name TEXT)");
    db.run("CREATE TABLE orders (id INT, user_id INT)");
    db.run("INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')");
    db.run("INSERT INTO orders VALUES (100, 1), (101, 2)");

    let result = db.run("SELECT users.name, orders.id FROM users LEFT JOIN orders ON users.id = orders.user_id ORDER BY users.id");
    assert_eq!(result.rows.len(), 3);
    // Carol has no orders, so orders.id should be NULL
    assert_eq!(result.rows[2][0], Value::Text("Carol".to_string()));
    assert_eq!(result.rows[2][1], Value::Null);
}

#[test]
fn test_arithmetic_in_select() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE items (price INT, qty INT)");
    db.run("INSERT INTO items VALUES (10, 5), (20, 3)");

    let result = db.run("SELECT price * qty FROM items ORDER BY price");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Int4(50)); // 10 * 5
    assert_eq!(result.rows[1][0], Value::Int4(60)); // 20 * 3
}

#[test]
fn test_string_like() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("SELECT name FROM users WHERE name LIKE 'A%'");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Text("Alice".to_string()));

    let result = db.run("SELECT name FROM users WHERE name LIKE '%o%'");
    assert_eq!(result.rows.len(), 2); // Bob, Carol
}

#[test]
fn test_numeric_comparison_coercion() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE prices (id INT, price FLOAT)");
    db.run("INSERT INTO prices VALUES (1, 9.99), (2, 19.99)");

    // Compare FLOAT column with INT literal
    let result = db.run("SELECT id FROM prices WHERE price > 15");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(2));
}

#[test]
fn test_string_to_int_coercion() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    // String literal '2' should coerce to int for comparison
    let result = db.run("SELECT name FROM users WHERE id = '2'");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Text("Bob".to_string()));
}

#[test]
fn test_table_not_found() {
    let mut db = TestDb::new();
    let result = db.try_run("SELECT * FROM nonexistent");
    assert!(result.is_err());
}

#[test]
fn test_column_not_found() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE test (id INT)");
    let result = db.try_run("SELECT nonexistent FROM test");
    assert!(result.is_err());
}

#[test]
fn test_duplicate_table() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE test (id INT)");
    let result = db.try_run("CREATE TABLE test (id INT)");
    assert!(result.is_err());
}

/// A target-less `ON CONFLICT DO NOTHING` arbitrates against every unique / PK
/// constraint: a conflicting row is skipped (not a `23505`), a fresh row inserts.
#[test]
fn test_on_conflict_do_nothing_no_target() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (id INT PRIMARY KEY, v TEXT)");
    db.run("INSERT INTO t (id, v) VALUES (1, 'a')");

    // VALUES form: PK conflict → skip, original untouched.
    db.run("INSERT INTO t (id, v) VALUES (1, 'b') ON CONFLICT DO NOTHING");
    let rows = db.run("SELECT id, v FROM t");
    assert_eq!(rows.rows.len(), 1);
    assert_eq!(rows.rows[0][1], Value::Text("a".into()));

    // INSERT … SELECT form: same skip.
    db.run("INSERT INTO t (id, v) SELECT 1, 'c' ON CONFLICT DO NOTHING");
    assert_eq!(db.run("SELECT count(*) FROM t").rows[0][0], Value::Int8(1));

    // A non-conflicting row still inserts.
    db.run("INSERT INTO t (id, v) VALUES (2, 'z') ON CONFLICT DO NOTHING");
    assert_eq!(db.run("SELECT count(*) FROM t").rows[0][0], Value::Int8(2));
}
