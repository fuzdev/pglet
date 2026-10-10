//! Engine integration tests: subqueries, CTEs, EXISTS/correlated subqueries,
//! and alias-aware column scoping.

use crate::common;
use common::*;
use pglet_sql::parser::parse;
use pglet_sql::types::{TypeOid, Value, value_to_text};

#[test]
fn test_self_join_aliased_collision() {
    let mut db = TestDb::new();
    // A self-join where both aliases expose the same column names (`id`/`parent`).
    // The effective-name (alias) plumbing on `Plan::Scan` keys the `JoinColumnMap`
    // on `a`/`b`, so `a.id` and `b.id` resolve to distinct sides instead of
    // colliding. `a` is the parent row, `b` the child row.
    db.run("CREATE TABLE node (id INT, parent INT)");
    db.run("INSERT INTO node VALUES (1, NULL), (2, 1), (3, 1)");

    let result = db.run(
        "SELECT a.id, b.id FROM node a INNER JOIN node b ON a.id = b.parent \
         ORDER BY a.id, b.id",
    );
    assert_eq!(result.rows.len(), 2);
    // Parent 1 has two children (2 and 3); `a.id` is the parent, `b.id` the child.
    assert_eq!(result.rows[0], vec![Value::Int4(1), Value::Int4(2)]);
    assert_eq!(result.rows[1], vec![Value::Int4(1), Value::Int4(3)]);
}

#[test]
fn test_join_on_eval_error_propagates() {
    let mut db = TestDb::new();
    // The nested-loop join's ON predicate is correct-or-loud: an evaluation error
    // propagates instead of being silently swallowed (the old `matches!(…,
    // Ok(Bool(true)))` matched the error to "no row", turning an unevaluable ON
    // into a silent 0-row result). `LOWER(t1.n)` on an integer column errors at
    // eval, so the join must fail loud rather than return zero rows.
    db.run("CREATE TABLE t1 (id INT, n INT)");
    db.run("CREATE TABLE t2 (id INT, s TEXT)");
    db.run("INSERT INTO t1 VALUES (1, 5)");
    db.run("INSERT INTO t2 VALUES (1, 'x')");

    let result = db.try_run("SELECT t1.id FROM t1 JOIN t2 ON LOWER(t1.n) = t2.s");
    assert!(
        result.is_err(),
        "an ON-predicate eval error must propagate, got {result:?}"
    );
}

#[test]
fn test_multi_table_join_alias_resolution() {
    let mut db = TestDb::new();
    // The spine's audit-log `list_with_usernames` shape: a left-deep chain of LEFT
    // JOINs (`Join(Join(Join(al, oa), oaa), od)`) with the `account` table aliased
    // twice (`oaa` via the actor chain, `od` via the direct account_id) plus a
    // COALESCE preferring the actor chain. A nested JOIN's output columns arrive
    // already qualified (`al.account_id`); they must stay registered under their
    // original aliases so a qualified ref two joins deep resolves, instead of being
    // re-prefixed under the `"joined"` placeholder (which made every chained ON/
    // projection ref fail → NULL usernames).
    db.run("CREATE TABLE account (id INT, username TEXT)");
    db.run("CREATE TABLE actor (id INT, account_id INT, name TEXT)");
    db.run("CREATE TABLE audit_log (seq INT, actor_id INT, account_id INT)");
    db.run("INSERT INTO account VALUES (10, 'wu_chain'), (20, 'wu_direct')");
    db.run("INSERT INTO actor VALUES (1, 10, 'A')");
    // row 100: has an actor (→ account 10 'wu_chain') AND a direct account_id (20)
    //          — the chain is preferred. row 101: no actor, direct account 20 only.
    db.run("INSERT INTO audit_log VALUES (100, 1, 20), (101, NULL, 20)");

    let result = db.run(
        "SELECT al.seq, COALESCE(oaa.username, od.username) AS username \
         FROM audit_log al \
         LEFT JOIN actor oa ON oa.id = al.actor_id \
         LEFT JOIN account oaa ON oaa.id = oa.account_id \
         LEFT JOIN account od ON od.id = al.account_id \
         ORDER BY al.seq",
    );
    assert_eq!(result.columns()[1].name, "username");
    assert_eq!(
        result.rows,
        vec![
            // chain wins over the direct account
            vec![Value::Int4(100), Value::Text("wu_chain".to_string())],
            // no actor chain → falls back to the direct account
            vec![Value::Int4(101), Value::Text("wu_direct".to_string())],
        ]
    );
}

#[test]
fn test_param_typed_via_alias_qualified_sibling() {
    let mut db = TestDb::new();
    // A `$N` compared against an ALIAS-qualified column inside a correlated EXISTS
    // must be typed from that column. Before alias-aware `infer_type`, `g.grantee`
    // resolved against the non-existent table `g` and the param stayed untyped
    // (which then drove the client's type-introspection into a loop). The
    // qualifier now falls back to the scan's real table, so `$1` types as `uuid`.
    db.run("CREATE TABLE cell (id UUID PRIMARY KEY)");
    db.run("CREATE TABLE cell_grant (id UUID PRIMARY KEY, cell_id UUID NOT NULL, grantee UUID NOT NULL)");

    let sql = "SELECT c.id FROM cell c \
               WHERE EXISTS (SELECT 1 FROM cell_grant g WHERE g.cell_id = c.id AND g.grantee = $1)";
    let p = parse(sql, &db.schema).expect("parse").remove(0);
    let types = pglet_sql::planner::infer_param_types(&p, &db.schema, &[None]).expect("infer");
    assert_eq!(types, vec![Some(TypeOid::Uuid)]);
}

#[test]
fn test_any_array_param_typed_across_join() {
    let mut db = TestDb::new();
    // The spine's `actor_lookup` shape: `WHERE act.id = ANY($1)` with a BARE (uncast)
    // array param, over a JOIN. The element type must be inferred from the qualified
    // `act.id`, which needs the FROM-scope alias map (`act` → `actor`) — a single
    // `table` context can't resolve an alias across a multi-table join. Without it
    // `$1` stays untyped and decodes as text, then `ANY` fails loud ("requires an
    // array on the right-hand side").
    db.run("CREATE TABLE account (id UUID PRIMARY KEY, username TEXT)");
    db.run("CREATE TABLE actor (id UUID PRIMARY KEY, account_id UUID NOT NULL, name TEXT)");

    let sql = "SELECT act.id, a.username, act.name AS display_name \
               FROM actor act JOIN account a ON a.id = act.account_id \
               WHERE act.id = ANY($1)";
    let p = parse(sql, &db.schema).expect("parse").remove(0);
    let types = pglet_sql::planner::infer_param_types(&p, &db.schema, &[None]).expect("infer");
    assert_eq!(types, vec![Some(TypeOid::UuidArray)]);
}

#[test]
fn test_scalar_subquery() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE products (id INT, price INT)");
    db.run("INSERT INTO products VALUES (1, 10), (2, 20), (3, 30)");

    let result = db.run("SELECT id, (SELECT MAX(price) FROM products) FROM products ORDER BY id");
    assert_eq!(result.rows.len(), 3);
    // All rows should have max_price = 30
    assert_eq!(result.rows[0][1], Value::Int4(30));
    assert_eq!(result.rows[1][1], Value::Int4(30));
    assert_eq!(result.rows[2][1], Value::Int4(30));
}

#[test]
fn test_in_subquery() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE users (id INT, name TEXT)");
    db.run("CREATE TABLE active_ids (user_id INT)");
    db.run("INSERT INTO users VALUES (1, 'Alice'), (2, 'Bob'), (3, 'Carol')");
    db.run("INSERT INTO active_ids VALUES (1), (3)");

    let result =
        db.run("SELECT name FROM users WHERE id IN (SELECT user_id FROM active_ids) ORDER BY name");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("Alice".to_string()));
    assert_eq!(result.rows[1][0], Value::Text("Carol".to_string()));
}

#[test]
fn test_order_by_non_projected_column() {
    let mut db = TestDb::new();
    // ORDER BY a column that is NOT in the SELECT list. With the Sort above the
    // Project the key `created_at` would resolve to nothing (the projection dropped
    // it) and the sort would silently no-op, returning insertion order. The planner
    // instead sorts the pre-projection rows (which still carry `created_at`), so the
    // key drives the order — output is still just `id`.
    db.run("CREATE TABLE auth_session (id INT, created_at INT)");
    db.run("INSERT INTO auth_session VALUES (1, 100), (2, 300), (3, 200)");

    // DESC by created_at: 300(id2), 200(id3), 100(id1).
    let result = db.run("SELECT id FROM auth_session ORDER BY created_at DESC");
    assert_eq!(
        result.columns().len(),
        1,
        "trimmed back to one output column"
    );
    assert_eq!(result.columns()[0].name, "id");
    assert_eq!(
        result.rows,
        vec![
            vec![Value::Int4(2)],
            vec![Value::Int4(3)],
            vec![Value::Int4(1)],
        ]
    );

    // ASC is the reverse — proves the key really drives the order (not insertion).
    let result = db.run("SELECT id FROM auth_session ORDER BY created_at ASC");
    assert_eq!(
        result.rows,
        vec![
            vec![Value::Int4(1)],
            vec![Value::Int4(3)],
            vec![Value::Int4(2)],
        ]
    );

    // With OFFSET (no LIMIT): skip the newest, keep the rest in DESC order.
    let result = db.run("SELECT id FROM auth_session ORDER BY created_at DESC OFFSET 1");
    assert_eq!(
        result.rows,
        vec![vec![Value::Int4(3)], vec![Value::Int4(1)]]
    );
}

#[test]
fn test_delete_in_subquery_eviction() {
    let mut db = TestDb::new();
    // The spine's session/token eviction shape: keep the newest N rows per account,
    // delete the rest, via `DELETE … WHERE id IN (SELECT id … ORDER BY created_at
    // DESC OFFSET N)`. Exercises both the DELETE-filter subquery wiring and the
    // ORDER-BY-non-projected carry-through.
    db.run("CREATE TABLE auth_session (id INT, account_id INT, created_at INT)");
    db.run("INSERT INTO auth_session VALUES (1,7,100),(2,7,200),(3,7,300),(4,8,150)");

    // Keep the newest 1 session for account 7 (id 3, created_at 300); delete the
    // two older ones (ids 1 and 2). Account 8's session is untouched.
    let result = db.run(
        "DELETE FROM auth_session \
         WHERE id IN (SELECT id FROM auth_session WHERE account_id = 7 \
                      ORDER BY created_at DESC OFFSET 1) \
         RETURNING id",
    );
    let mut deleted: Vec<i32> = result
        .rows
        .iter()
        .map(|r| match &r[0] {
            Value::Int4(i) => *i,
            other => panic!("unexpected RETURNING value {other:?}"),
        })
        .collect();
    deleted.sort_unstable();
    assert_eq!(
        deleted,
        vec![1, 2],
        "deletes the two oldest account-7 sessions"
    );

    // Survivors: account 7 keeps only its newest (id 3); account 8 untouched (id 4).
    let remaining = db.run("SELECT id FROM auth_session ORDER BY id");
    assert_eq!(
        remaining.rows,
        vec![vec![Value::Int4(3)], vec![Value::Int4(4)]]
    );
}

#[test]
fn test_cte_basic() {
    let mut db = TestDb::new();
    setup_users(&mut db);

    let result = db.run("WITH active_users AS (SELECT * FROM users WHERE active = true) SELECT name FROM active_users ORDER BY name");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("Alice".to_string()));
    assert_eq!(result.rows[1][0], Value::Text("Carol".to_string()));
}

#[test]
fn test_exists_and_not_exists() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE fact (hash TEXT, size INT)");
    db.run("INSERT INTO fact (hash, size) VALUES ('blake3:a', 1), ('blake3:b', 2)");

    // No-FROM projection EXISTS — the `query_has_fact` shape (routes through
    // `Plan::Values`, exercising the subquery-capable VALUES path). Unaliased
    // EXISTS is named `exists`.
    let result = db.run("SELECT EXISTS(SELECT 1 FROM fact WHERE hash = 'blake3:a') AS exists");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Bool(true));
    assert_eq!(result.columns()[0].name, "exists");
    assert_eq!(result.columns()[0].type_oid, TypeOid::Bool);

    // EXISTS is false when the inner query matches no rows.
    let result = db.run("SELECT EXISTS(SELECT 1 FROM fact WHERE hash = 'blake3:missing')");
    assert_eq!(result.rows[0][0], Value::Bool(false));
    assert_eq!(result.columns()[0].name, "exists");

    // NOT EXISTS is the negation.
    let result = db.run("SELECT NOT EXISTS(SELECT 1 FROM fact WHERE hash = 'blake3:missing')");
    assert_eq!(result.rows[0][0], Value::Bool(true));
    let result = db.run("SELECT NOT EXISTS(SELECT 1 FROM fact WHERE hash = 'blake3:a')");
    assert_eq!(result.rows[0][0], Value::Bool(false));

    // EXISTS with a constant inner query (no FROM at all on the inner side).
    let result = db.run("SELECT EXISTS(SELECT 1) AS e");
    assert_eq!(result.rows[0][0], Value::Bool(true));

    // Non-correlated EXISTS in a WHERE clause with an outer FROM: the inner
    // query is constant, so it keeps every outer row when it matches, none
    // otherwise. A multi-column inner projection (`SELECT *`) is fine — only
    // row presence matters.
    let result = db
        .run("SELECT hash FROM fact WHERE EXISTS(SELECT * FROM fact WHERE size = 2) ORDER BY hash");
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("blake3:a".to_string()));
    assert_eq!(result.rows[1][0], Value::Text("blake3:b".to_string()));

    // NOT EXISTS in WHERE: the inner query has no match, so all outer rows pass.
    let result = db.run(
        "SELECT hash FROM fact WHERE NOT EXISTS(SELECT 1 FROM fact WHERE size = 99) ORDER BY hash",
    );
    assert_eq!(result.rows.len(), 2);

    // WHERE EXISTS with an empty inner result drops every outer row.
    let result = db.run("SELECT hash FROM fact WHERE EXISTS(SELECT 1 FROM fact WHERE size = 99)");
    assert_eq!(result.rows.len(), 0);
}

#[test]
fn test_correlated_exists() {
    let mut db = TestDb::new();
    // The orphan-fact spine shape (fuz_app's `db/fact_queries.ts`), reduced: a fact
    // is orphan iff no live cell references its hash. The inner subquery correlates
    // on the outer `f.hash`.
    db.run("CREATE TABLE fact (hash TEXT, size INT)");
    db.run("CREATE TABLE cell (refs TEXT, deleted BOOL)");
    db.run("INSERT INTO fact (hash, size) VALUES ('blake3:orphan', 10), ('blake3:live', 20), ('blake3:dead', 30)");
    // A live cell references blake3:live; a soft-deleted cell references blake3:dead.
    db.run("INSERT INTO cell (refs, deleted) VALUES ('blake3:live', false), ('blake3:dead', true)");

    // (a) Correlated NOT EXISTS returns only the unreferenced outer rows — the
    // orphan (no cell at all) and the dead-referenced fact (its only cell is
    // soft-deleted), but not the live-referenced fact.
    let result = db.run(
        "SELECT hash FROM fact f \
         WHERE NOT EXISTS (SELECT 1 FROM cell c WHERE c.refs = f.hash AND c.deleted = false) \
         ORDER BY hash",
    );
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Text("blake3:dead".to_string()));
    assert_eq!(result.rows[1][0], Value::Text("blake3:orphan".to_string()));

    // (b) Correlated EXISTS returns only the referenced outer rows (the inverse) —
    // here only the live-referenced fact has a matching live cell.
    let result = db.run(
        "SELECT hash FROM fact f \
         WHERE EXISTS (SELECT 1 FROM cell c WHERE c.refs = f.hash AND c.deleted = false) \
         ORDER BY hash",
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Text("blake3:live".to_string()));

    // (c) Empty inner table: NOT EXISTS keeps every outer row, EXISTS drops them all.
    db.run("CREATE TABLE empty_cell (refs TEXT)");
    let result = db.run(
        "SELECT hash FROM fact f \
         WHERE NOT EXISTS (SELECT 1 FROM empty_cell c WHERE c.refs = f.hash) ORDER BY hash",
    );
    assert_eq!(result.rows.len(), 3);
    let result = db.run(
        "SELECT hash FROM fact f \
         WHERE EXISTS (SELECT 1 FROM empty_cell c WHERE c.refs = f.hash)",
    );
    assert_eq!(result.rows.len(), 0);
}

#[test]
fn test_correlated_exists_aliased() {
    let mut db = TestDb::new();
    // (d) Alias-shadowed correlated EXISTS (the cell-authz shape): the outer
    // `c.id` and the inner `g.cell_id` both reference an `id`-named column, and
    // the inner `cell_grant` ALSO has its own `id`. Alias-aware scoping resolves
    // `c.id` to the OUTER `cell c` (qualifier `c` ≠ the inner alias `g`) while
    // `g.cell_id` stays inner — so the query authz-filters correctly instead of
    // failing loud. Cell 1 has a matching grant (cell_id = 1); cell 2 does not.
    db.run("CREATE TABLE cell (id INT)");
    db.run("CREATE TABLE cell_grant (id INT, cell_id INT)");
    db.run("INSERT INTO cell VALUES (1), (2)");
    db.run("INSERT INTO cell_grant VALUES (10, 1)");

    let result = db.run(
        "SELECT id FROM cell c WHERE EXISTS (SELECT 1 FROM cell_grant g WHERE g.cell_id = c.id) \
         ORDER BY id",
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(1));

    // The inverse (NOT EXISTS) returns the un-granted cell.
    let result = db.run(
        "SELECT id FROM cell c WHERE NOT EXISTS (SELECT 1 FROM cell_grant g WHERE g.cell_id = c.id) \
         ORDER BY id",
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(2));
}

#[test]
fn test_nested_correlated_exists() {
    let mut db = TestDb::new();
    // Doubly-nested correlation exercises the outer-frame *chain*: the innermost
    // subquery references BOTH its immediate parent (`b.id`) AND the outermost
    // scope two levels up (`a.tag`), so eval-time resolution must walk past the
    // `b` frame to reach the `a` frame. (Before A2, each enclosing level's
    // substitution pass bound its own refs into the nested plan; now a single env
    // threads a frame chain.)
    db.run("CREATE TABLE a (id INT, tag TEXT)");
    db.run("CREATE TABLE b (id INT, a_id INT)");
    db.run("CREATE TABLE c (id INT, b_id INT, a_tag TEXT)");
    db.run("INSERT INTO a VALUES (1, 'x'), (2, 'y')");
    db.run("INSERT INTO b VALUES (10, 1), (20, 2)");
    // The one `c` row links b=10 and carries a_tag 'x' — so it matches only the
    // a=1 chain (b.a_id = 1, a.tag = 'x'); a=2 has no qualifying inner row.
    db.run("INSERT INTO c VALUES (100, 10, 'x')");

    let result = db.run(
        "SELECT a.id FROM a \
         WHERE EXISTS ( \
           SELECT 1 FROM b WHERE b.a_id = a.id AND EXISTS ( \
             SELECT 1 FROM c WHERE c.b_id = b.id AND c.a_tag = a.tag \
           ) \
         ) ORDER BY a.id",
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(1));
}

#[test]
fn test_alias_aware_resolution() {
    let mut db = TestDb::new();
    // A FROM-clause alias resolves a qualified reference in both the projection
    // and the WHERE predicate (`s.x` over `FROM t s`).
    db.run("CREATE TABLE t (x INT, y INT)");
    db.run("INSERT INTO t VALUES (1, 10), (2, 20)");

    let result = db.run("SELECT s.x FROM t s WHERE s.x = 1");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(1));

    // A reference qualified by the real table name still resolves when no alias
    // is given.
    let result = db.run("SELECT t.y FROM t WHERE t.x = 2");
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][0], Value::Int4(20));
}

#[test]
fn test_any_array_param_typed_in_join_on_under_distinct() {
    let mut db = TestDb::new();
    // The spine's scoped `query_actor_search` shape: a 3-table join whose
    // `role_grant` join carries `… AND rg.scope_id = ANY($2) …` conjuncts in its
    // ON clause, under `SELECT DISTINCT`. The bare uuid-array `$2` element type
    // must be inferred from the alias-qualified `rg.scope_id` via the FROM-scope
    // alias map. The regression: `collect_from_aliases` didn't pass through a
    // `Plan::Distinct` node, so for a `SELECT DISTINCT` the FROM-scope map came
    // back empty, `rg` couldn't resolve, and `$2` stayed untyped — decoded as
    // text, then `ANY` failed loud and the swallowing join matched **0 rows**.
    db.run("CREATE TABLE account (id UUID PRIMARY KEY, username TEXT NOT NULL)");
    db.run(
        "CREATE TABLE actor (id UUID PRIMARY KEY, account_id UUID NOT NULL, name TEXT NOT NULL)",
    );
    db.run(
        "CREATE TABLE role_grant (id UUID PRIMARY KEY, actor_id UUID NOT NULL, \
         scope_id UUID, revoked_at TIMESTAMPTZ, expires_at TIMESTAMPTZ)",
    );

    let sql = "SELECT DISTINCT act.id, a.username, act.name AS display_name \
               FROM actor act \
               JOIN account a ON a.id = act.account_id \
               JOIN role_grant rg ON rg.actor_id = act.id \
                 AND rg.scope_id = ANY($2) \
                 AND rg.revoked_at IS NULL \
                 AND (rg.expires_at IS NULL OR rg.expires_at > NOW()) \
               WHERE LOWER(act.name) LIKE $1 || '%' ESCAPE '\\' \
               ORDER BY display_name, id \
               LIMIT $3";
    let p = parse(sql, &db.schema).expect("parse").remove(0);
    let types =
        pglet_sql::planner::infer_param_types(&p, &db.schema, &[None, None, None]).expect("infer");
    // `$2` is the join-ON uuid-array param (the regression target); `$3` is the
    // bigint LIMIT. `$1` rides a `||` string concat → text (leaving it untyped
    // reports OID 0, which loops a binary-protocol client's type introspection).
    assert_eq!(types[0], Some(TypeOid::Text));
    assert_eq!(types[1], Some(TypeOid::UuidArray));
    assert_eq!(types[2], Some(TypeOid::Int8));
}

#[test]
fn test_scoped_actor_search_join_distinct_returns_rows() {
    let mut db = TestDb::new();
    // The same scoped-search join/DISTINCT/ORDER-BY shape, executed with a literal
    // scope array (no params) — a regression guard that the join's ON conjuncts,
    // the `DISTINCT` collapse of an actor's multiple matching grants, and the
    // ORDER-BY-on-projected-alias all hold once the param types resolve.
    db.run("CREATE TABLE account (id UUID PRIMARY KEY, username TEXT NOT NULL)");
    db.run(
        "CREATE TABLE actor (id UUID PRIMARY KEY, account_id UUID NOT NULL, name TEXT NOT NULL)",
    );
    db.run(
        "CREATE TABLE role_grant (id UUID PRIMARY KEY, actor_id UUID NOT NULL, \
         scope_id UUID, revoked_at TIMESTAMPTZ, expires_at TIMESTAMPTZ)",
    );
    db.run(
        "INSERT INTO account (id, username) VALUES \
         ('00000000-0000-0000-0000-0000000000a1', 'alice')",
    );
    db.run(
        "INSERT INTO actor (id, account_id, name) VALUES \
         ('00000000-0000-0000-0000-000000000011', '00000000-0000-0000-0000-0000000000a1', 'Alice')",
    );
    // Two active grants on the same scope for one actor — DISTINCT must collapse
    // them to a single row.
    db.run(
        "INSERT INTO role_grant (id, actor_id, scope_id, revoked_at, expires_at) VALUES \
         ('00000000-0000-0000-0000-0000000000f1', '00000000-0000-0000-0000-000000000011', \
          '00000000-0000-0000-0000-0000000000c0', NULL, NULL), \
         ('00000000-0000-0000-0000-0000000000f2', '00000000-0000-0000-0000-000000000011', \
          '00000000-0000-0000-0000-0000000000c0', NULL, NULL)",
    );

    let result = db.run(
        "SELECT DISTINCT act.id, a.username, act.name AS display_name \
         FROM actor act \
         JOIN account a ON a.id = act.account_id \
         JOIN role_grant rg ON rg.actor_id = act.id \
           AND rg.scope_id = ANY(ARRAY['00000000-0000-0000-0000-0000000000c0'::uuid]) \
           AND rg.revoked_at IS NULL \
           AND (rg.expires_at IS NULL OR rg.expires_at > NOW()) \
         WHERE LOWER(act.name) LIKE 'a' || '%' ESCAPE '\\' \
         ORDER BY display_name, id",
    );
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0][2], Value::Text("Alice".to_string()));
}

/// A set-op column that names something no query level in the statement resolves is never
/// unified, so the statement's second parse leaves it pending again — and a plan holding a
/// provisional column type is refused rather than returned. 18.6 answers `42703` for the
/// name itself, which pglet's `VALUES` list has no namespace to reach.
#[test]
fn test_never_unified_set_op_column_is_refused_at_parse() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE nus (n NUMERIC)");
    let err = parse(
        "INSERT INTO nus VALUES ((SELECT zz EXCEPT SELECT 1))",
        &db.schema,
    )
    .expect_err("a never-unified set-op column must not reach a returned plan");
    assert_eq!(err.sqlstate(), "0A000");
}

/// A branch whose subquery projects a **wildcard** is typed from what that wildcard
/// stands over — a scan's catalog column, or a derived table's own query's — so the
/// branch set unifies to `numeric` and the executor's walker declares it. Left
/// uncertain, both the walker and the values came from whichever branch each reached
/// first, and the row that took the `integer` branch carried an int4 value under the
/// projection's `numeric` declaration.
#[test]
fn test_wildcard_subquery_branch_unifies_the_set() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE wsb (c NUMERIC(6,2))");
    db.run("INSERT INTO wsb VALUES (1.50)");
    db.run("CREATE TABLE wsk (id INT)");
    db.run("INSERT INTO wsk VALUES (1), (2)");

    let result =
        db.run("SELECT COALESCE((SELECT * FROM wsb WHERE c > k.id), 2) AS v FROM wsk k ORDER BY 1");
    assert_eq!(result.columns()[0].type_oid, TypeOid::Numeric);
    // Both rows are numeric values, not one numeric and one int4 under that declaration.
    assert!(
        result
            .rows
            .iter()
            .all(|row| matches!(row[0], Value::Numeric(_))),
        "every row of this branch set carries the type the set unified to: {:?}",
        result.rows
    );

    // The derived-table spelling asks the same question of the derived table's own query.
    let result = db.run("SELECT COALESCE((SELECT * FROM (SELECT o.c AS z) w), 2) AS v FROM wsb o");
    assert_eq!(result.columns()[0].type_oid, TypeOid::Numeric);
    assert!(matches!(result.rows[0][0], Value::Numeric(_)));
}

/// A branch the coercion pass cannot **wrap** — `name` is no cast's target, and a `Cast`
/// erases an enum's identity — is still a type the gate *reads*, so the set is **judged**
/// rather than answered under one branch's type. Judged, not refused: `name` carries its
/// string-category `pg_cast` rows into the fold, so the same subquery beside a `text`
/// sibling resolves. These are the pairs 18.6 refuses, and the enum's message now names
/// the **enum** as 18.6 does, `SubqueryType::Uncastable` carrying the identity beside the
/// OID (`EDGES.md` row 152's enum member, closed with row 114). And the refusal a bare
/// subquery once hid was its siblings': `integer` and `date` have no common type whatever
/// the subquery's is.
#[test]
fn test_uncastable_subquery_branch_is_gated_not_deferred() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE ucb (i INT, d DATE)");
    db.run("INSERT INTO ucb VALUES (1, '2020-01-02')");
    db.run("CREATE TYPE ucmood AS ENUM ('sad', 'ok')");
    db.run("CREATE TABLE ucm (m ucmood)");
    db.run("INSERT INTO ucm VALUES ('ok')");
    db.run("CREATE TABLE uce (c INT)");

    for sql in [
        "SELECT COALESCE((SELECT current_user), i, d) FROM ucb",
        "SELECT COALESCE((SELECT m FROM ucm), i, d) FROM ucb",
        "SELECT CASE WHEN true THEN (SELECT current_user) WHEN false THEN i ELSE d END FROM ucb",
        // The subquery is not even half of the disagreeing pair here.
        "SELECT COALESCE((SELECT * FROM uce), i, d) FROM ucb",
    ] {
        let err = db
            .try_run(sql)
            .expect_err("a branch set with no common type is refused");
        assert_eq!(err.sqlstate(), "42804", "{sql}");
    }
}

/// Two occurrences of one pending set-op subquery are the same expression: the record
/// each carries is the parser's channel to the statement's second parse and no part of
/// the plan's identity, so a `SELECT DISTINCT` sort key still matches the target entry it
/// equals. The key's own node is then discarded, and the hints it never earned are the
/// entry's.
#[test]
fn test_pending_set_op_subquery_written_twice_matches_itself() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE psq (i INT)");
    db.run("INSERT INTO psq VALUES (7), (9)");

    let result = db.run(
        "SELECT DISTINCT (SELECT i EXCEPT SELECT '7') AS z FROM psq \
         ORDER BY (SELECT i EXCEPT SELECT '7')",
    );
    assert_eq!(result.columns()[0].type_oid, TypeOid::Int4);
    assert_eq!(result.rows.len(), 2);
    assert_eq!(result.rows[0][0], Value::Int4(9));
    assert_eq!(result.rows[1][0], Value::Null);
}

/// An exact `sum`/`avg` reads every row's value or fails loud — never skips one. A
/// `CASE`/`COALESCE` branch over a function-scan subquery is left unconverted
/// (`EDGES.md` row 152), so a column declared one type carries another's values. Where
/// the declared type is `numeric` and the stray values are integers, the integer is read
/// exactly, as 18.6's conversion would. Where it is `integer`, the type is a guess the
/// derived column carries and not a certain one, so `avg` keeps `double precision` and
/// averages the `numeric` value; `sum` keeps `bigint`, where a `numeric` value has no
/// exact reading and is refused `0A000` — it used to be skipped, answering `3` for
/// 18.6's `4.5`.
#[test]
fn an_exact_aggregate_reads_or_refuses_an_unconverted_branch_value() {
    let mut db = TestDb::new();
    let numeric_first = db.run(
        "SELECT sum(n), avg(n) FROM (SELECT CASE WHEN x IS NULL \
         THEN (SELECT * FROM unnest(ARRAY[1.5::numeric])) ELSE x END AS n \
         FROM (VALUES (1), (NULL), (2)) v(x)) s",
    );
    let shown: Vec<String> = numeric_first.rows[0]
        .iter()
        .map(|v| match v {
            Value::Numeric(n) => n.to_display_string(),
            other => panic!("expected numeric, got {other:?}"),
        })
        .collect();
    assert_eq!(shown, ["4.5", "1.5000000000000000"]);
    let guessed = "FROM (SELECT COALESCE(x, (SELECT * FROM unnest(ARRAY[1.5::numeric]))) AS n \
                   FROM (VALUES (1), (NULL), (2)) v(x)) s";
    let err = db
        .try_run(&format!("SELECT sum(n) {guessed}"))
        .expect_err("an integer-declared column holding a numeric must be loud under sum");
    assert_eq!(err.sqlstate(), "0A000", "{err}");
    let avg = db.run(&format!("SELECT avg(n) {guessed}"));
    assert_eq!(avg.columns()[0].type_oid, TypeOid::Float8);
    assert_eq!(avg.rows[0][0], Value::Float8(1.5));
}

/// The tables [`AGGREGATE_SHAPES`] and the certainty tests read. `t.b` holds a value past
/// 2^53 beside small ones, which an exact accumulation and a `float8` one tell apart.
fn aggregate_shapes_db() -> TestDb {
    let mut db = TestDb::new();
    db.run(
        "CREATE TABLE t (k INT, i INT, s SMALLINT, b BIGINT, f FLOAT8, n NUMERIC, tx TEXT, \
         d DATE, d2 DATE)",
    );
    db.run(
        "INSERT INTO t VALUES (1, 1, 1, 1, 1.5, 1.25, 'a', '2020-01-02', '2020-01-01'), \
         (2, NULL, NULL, NULL, 2.5, 2.5, 'bb', NULL, NULL), \
         (3, 5, 5, 9007199254740993, NULL, NULL, NULL, '2021-03-04', '2021-03-01'), \
         (4, 2, 2, 2, NULL, NULL, 'cccc', '2020-01-05', '2020-01-01')",
    );
    db.run("CREATE TABLE u (k INT, i INT, b BIGINT)");
    db.run("INSERT INTO u VALUES (1, 10, 10), (3, 30, 30), (5, 50, 50)");
    db.run("CREATE TABLE u8 (k BIGINT, z INT)");
    db.run("INSERT INTO u8 VALUES (1, 1), (3, 3)");
    db.run("CREATE TABLE tw (k INT, i INT, b BIGINT)");
    db.run("INSERT INTO tw VALUES (1, 1, 0)");
    // two columns: a correlated name over a one-column table is `EDGES.md` row 159's
    db.run("CREATE TABLE uw (j INT, b BIGINT)");
    db.run("INSERT INTO uw VALUES (1, 9007199254740992), (2, 1), (3, 1)");
    db
}

/// The declared types of `sql` on the extended protocol's `Describe` and on the simple
/// protocol, which must agree, beside the first row as text. A statement that does not
/// parse describes no column, and so fails the comparison.
fn declared_types_and_first_row(db: &mut TestDb, sql: &str) -> (Vec<TypeOid>, Vec<Option<String>>) {
    let describe: Vec<TypeOid> = parse(sql, &db.schema)
        .ok()
        .and_then(|plans| {
            plans.first().map(|plan| {
                plan.result_shape(&db.schema)
                    .columns()
                    .iter()
                    .map(|c| c.type_oid)
                    .collect()
            })
        })
        .unwrap_or_default();
    let result = db.run(sql);
    let simple: Vec<TypeOid> = result.columns().iter().map(|c| c.type_oid).collect();
    assert_eq!(describe, simple, "{sql}: Describe against the simple leg");
    let first = result
        .rows
        .first()
        .map(|row| row.iter().map(value_to_text).collect())
        .unwrap_or_default();
    (simple, first)
}

/// A relation's column is certain where the expression defining it is one the coercion
/// pass typed as written, and `avg` and `sum(bigint)` over it are then 18.6's exact
/// `numeric`: a `VALUES` column, a computed or aggregate entry of a derived table or a
/// `WITH` query, a set operation's column — an untyped `NULL` side included — a
/// recursive query's, a cast array's `unnest`, a `USING` merge of same-typed columns
/// passed through, a scalar subquery whose own column is certain — as the entry, as an
/// operand and as a branch of a fold — and a nesting of those. Each value is 18.6's,
/// measured, over data whose `float8` accumulation is another number.
#[test]
fn a_relations_column_is_certain_where_its_definition_was_typed_as_written() {
    let mut db = aggregate_shapes_db();
    let exact: &[(&str, &str, &str)] = &[
        (
            "SELECT avg(x), sum(x) FROM (VALUES (9007199254740993), (1), (1)) v(x)",
            "3002399751580331.6667",
            "9007199254740995",
        ),
        (
            "SELECT avg(x), sum(x) FROM (VALUES (1), (5000000000)) v(x)",
            "2500000000.50000000",
            "5000000001",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT b + 0 AS x FROM t) q",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT max(b) AS x FROM t GROUP BY k) q",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT count(*) AS x FROM t GROUP BY k % 2) q",
            "2.0000000000000000",
            "4",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT sum(i) AS x FROM t GROUP BY k % 2) q",
            "4.0000000000000000",
            "8",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT b AS x FROM t UNION ALL SELECT i FROM u) q",
            "1501199875790181.0000",
            "9007199254741086",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT b AS x FROM t UNION ALL SELECT NULL) q",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        (
            "SELECT avg(x), sum(x) FROM \
             (SELECT 9007199254740993 AS x UNION ALL SELECT 1 UNION ALL SELECT 1) q",
            "3002399751580331.6667",
            "9007199254740995",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT x * 1 AS x FROM (SELECT b + 0 AS x FROM t) q0) q",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        (
            "WITH c(x) AS (SELECT b + 0 FROM t) SELECT avg(x), sum(x) FROM c",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        (
            "WITH RECURSIVE r(x) AS (SELECT 9007199254740993 UNION ALL \
             SELECT x - 9007199254740992 FROM r WHERE x > 2) SELECT avg(x), sum(x) FROM r",
            "4503599627370497.0000",
            "9007199254740994",
        ),
        (
            "SELECT avg(x), sum(x) FROM unnest('{9007199254740993,1,1}'::bigint[]) q(x)",
            "3002399751580331.6667",
            "9007199254740995",
        ),
        // a scalar subquery whose sub-plan's column is certain: the entry itself, an
        // operand, a branch of a fold, a `VALUES` cell, a set operation's side
        (
            "SELECT avg(x), sum(x) FROM (SELECT (SELECT b FROM t t2 WHERE t2.k = t.k) AS x FROM t) q",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        (
            "SELECT avg(x), sum(x) FROM \
             (SELECT (SELECT max(b) FROM t t2 WHERE t2.k <= t.k) AS x FROM t) q",
            "4503599627370497.0000",
            "18014398509481988",
        ),
        (
            "SELECT avg(x), sum(x) FROM \
             (SELECT (SELECT b FROM t t2 WHERE t2.k = t.k) + 0 AS x FROM t) q",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT -(SELECT b FROM t t2 WHERE t2.k = t.k) AS x FROM t) q",
            "-3002399751580332.0000",
            "-9007199254740996",
        ),
        (
            "SELECT avg(x), sum(x) FROM \
             (SELECT COALESCE((SELECT b FROM t t2 WHERE t2.k = t.k), 1) AS x FROM t) q",
            "2251799813685249.2500",
            "9007199254740997",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT CASE WHEN k > 0 \
             THEN (SELECT b FROM t t2 WHERE t2.k = t.k) ELSE 0 END AS x FROM t) q",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        (
            "SELECT avg(x), sum(x) FROM \
             (SELECT greatest((SELECT b FROM t t2 WHERE t2.k = t.k), 1) AS x FROM t) q",
            "2251799813685249.2500",
            "9007199254740997",
        ),
        (
            "SELECT avg(x), sum(x) FROM (VALUES ((SELECT max(b) FROM t)), (1), (1)) v(x)",
            "3002399751580331.6667",
            "9007199254740995",
        ),
        (
            "SELECT avg(x), sum(x) FROM \
             (SELECT (SELECT max(b) FROM t) AS x UNION ALL SELECT 1 UNION ALL SELECT 1) q",
            "3002399751580331.6667",
            "9007199254740995",
        ),
        (
            "WITH c(x) AS (SELECT (SELECT b FROM t t2 WHERE t2.k = t.k) FROM t) \
             SELECT avg(x), sum(x) FROM c",
            "3002399751580332.0000",
            "9007199254740996",
        ),
        // a sub-plan the pass's branch reader does not type is its column's all the
        // same where no fold reads it
        (
            "SELECT avg(x), sum(x) FROM (SELECT \
             (SELECT * FROM unnest('{9007199254740993,1}'::bigint[]) LIMIT 1) AS x FROM t) q",
            "9007199254740993.0000",
            "36028797018963972",
        ),
        // which a fold reads as a typed branch: the sub-plan's record is asked before the
        // branch reader that declines the wildcard
        (
            "SELECT avg(x), sum(x) FROM (SELECT COALESCE(\
             (SELECT * FROM unnest('{9007199254740993,1}'::bigint[]) LIMIT 1), 1) AS x FROM t) q",
            "9007199254740993.0000",
            "36028797018963972",
        ),
        // a subquery standing as an operand under a fold's branch
        (
            "SELECT avg(x), sum(x) FROM \
             (SELECT COALESCE(b, (SELECT max(b) FROM t) + 0) AS x FROM t) q",
            "4503599627370497.2500",
            "18014398509481989",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT greatest(-(SELECT 5000000000), b) AS x FROM t) q",
            "2251798563685249.0000",
            "9007194254740996",
        ),
    ];
    for (sql, avg, sum) in exact {
        let (types, row) = declared_types_and_first_row(&mut db, sql);
        assert_eq!(types, [TypeOid::Numeric, TypeOid::Numeric], "{sql}");
        assert_eq!(
            row,
            [Some((*avg).to_string()), Some((*sum).to_string())],
            "{sql}"
        );
    }
    // `sum` of an `integer` column is `bigint` whatever its certainty; `avg` is the one
    // that tells
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT avg(k), sum(k) FROM (SELECT k FROM t JOIN u USING (k)) q",
    );
    assert_eq!(types, [TypeOid::Numeric, TypeOid::Int8]);
    assert_eq!(row, [Some("2.0000000000000000".into()), Some("4".into())]);
    // a `date` less a `date` is an `integer` — directly, as a derived column, and as a
    // scalar subquery's column under a fold
    for (sql, avg) in [
        (
            "SELECT avg(d - d2), sum(d - d2) FROM t",
            "2.6666666666666667",
        ),
        (
            "SELECT avg(x), sum(x) FROM (SELECT d - d2 AS x FROM t) q",
            "2.6666666666666667",
        ),
        (
            "SELECT avg(x), sum(x) FROM \
             (SELECT COALESCE((SELECT d - d2 FROM t t2 WHERE t2.k = t.k), 0) AS x FROM t) q",
            "2.0000000000000000",
        ),
    ] {
        let (types, row) = declared_types_and_first_row(&mut db, sql);
        assert_eq!(types, [TypeOid::Numeric, TypeOid::Int8], "{sql}");
        assert_eq!(row, [Some(avg.into()), Some("8".into())], "{sql}");
    }
    // a function over the exact `avg` rounds as `numeric` does, away from zero
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT round(avg(x)), round(avg(x), 1) FROM (VALUES (1), (4)) v(x)",
    );
    assert_eq!(types, [TypeOid::Numeric, TypeOid::Numeric]);
    assert_eq!(row, [Some("3".into()), Some("2.5".into())]);
    // and a fold beside a computed branch reads the column's type, so the branch keeps
    // its value where it was cast to the column's `integer`
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT COALESCE(v.x, t.f + 0), greatest(v.x, t.n * 1) FROM t, (VALUES (NULL::int)) v(x) \
         WHERE k = 1",
    );
    assert_eq!(types, [TypeOid::Float8, TypeOid::Numeric]);
    assert_eq!(row, [Some("1.5".into()), Some("1.25".into())]);
    // a column a scalar subquery defines is such a branch too: its `numeric` — the
    // exact `avg` of its sub-plan — folds with the computed `double precision` sibling
    // to `double precision`, where the sibling was cast to the column's `numeric`
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT COALESCE(x, t.f + 0), greatest(x, t.f + 0) FROM \
         (SELECT (SELECT avg(i) FROM u WHERE i > 99) AS x) q, t WHERE t.k = 1",
    );
    assert_eq!(types, [TypeOid::Float8, TypeOid::Float8]);
    assert_eq!(row, [Some("1.5".into()), Some("1.5".into())]);
    // and a function over its exact `avg` rounds as `numeric` does
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT round(avg(x)), round(avg(x), 1) FROM \
         (SELECT (SELECT i FROM t t2 WHERE t2.k = t.k) AS x FROM t WHERE k IN (1, 4)) q",
    );
    assert_eq!(types, [TypeOid::Numeric, TypeOid::Numeric]);
    assert_eq!(row, [Some("2".into()), Some("1.5".into())]);
}

/// A relation's column whose definition the coercion pass did not type as written keeps
/// the answers an argument of unknown type gets — `avg` `double precision`, `sum` of a
/// `bigint` `bigint` — which are the ones pglet gave before a relation's column could be
/// certain. Each is a guess 18.6 does not share or a shape the pass's reader does not
/// type: a fold with a branch the pass could not type — an operand subquery whose own
/// column is a guess — read directly, under a set operation, in a `VALUES` cell or as a
/// recursive query's seed; a window
/// function; an `ARRAY[…]` constructor under `unnest`, whose elements no pass converts; a
/// `USING` merge of an `integer` and a `bigint`, declared at the left side's type; a
/// recursive term's untyped `NULL`; a `RETURNING` column; an argument over an enclosing
/// query's column; and a scalar subquery whose own column is one of those — as the entry
/// and as a branch of a fold, where the cast the pass writes over the branch names a
/// type and the sub-plan vouches for none.
#[test]
fn a_relations_column_stays_uncertain_where_its_definition_was_not_typed() {
    let mut db = aggregate_shapes_db();
    let float_avg = |values: &[f64]| {
        let count = u32::try_from(values.len()).expect("a short list");
        value_to_text(&Value::Float8(
            values.iter().sum::<f64>() / f64::from(count),
        ))
    };
    // the fold declares `integer` and holds 3.5: `avg` reads the values the guess cast
    let guessed = "SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) AS x FROM t";
    let uncertain: &[(String, TypeOid, Option<String>)] = &[
        (
            format!("SELECT avg(x) FROM ({guessed}) q"),
            TypeOid::Float8,
            float_avg(&[1.0, 4.0, 5.0, 2.0]),
        ),
        (
            format!("SELECT avg(x) FROM (SELECT i AS x FROM t UNION ALL {guessed}) q"),
            TypeOid::Float8,
            float_avg(&[1.0, 5.0, 2.0, 1.0, 4.0, 5.0, 2.0]),
        ),
        (
            format!("SELECT avg(x) FROM ({guessed} UNION ALL SELECT i FROM t) q"),
            TypeOid::Float8,
            float_avg(&[1.0, 4.0, 5.0, 2.0, 1.0, 5.0, 2.0]),
        ),
        (
            "SELECT avg(x) FROM (VALUES (COALESCE(1, 2.5 + (SELECT * FROM unnest(ARRAY[1])))), \
             (4)) v(x)"
                .into(),
            TypeOid::Float8,
            float_avg(&[1.0, 4.0]),
        ),
        (
            "WITH RECURSIVE r(x) AS (SELECT COALESCE(1, 2.5 + (SELECT * FROM unnest(ARRAY[1]))) \
             UNION ALL \
             SELECT x + 1 FROM r WHERE x < 4) SELECT avg(x) FROM r"
                .into(),
            TypeOid::Float8,
            float_avg(&[1.0, 2.0, 3.0, 4.0]),
        ),
        (
            "WITH RECURSIVE r(x) AS (SELECT 1 UNION ALL SELECT NULL FROM r WHERE x < 2) \
             SELECT avg(x) FROM r"
                .into(),
            TypeOid::Float8,
            float_avg(&[1.0]),
        ),
        // a scalar subquery over a guess, the entry itself and a fold's branch: a fold the
        // sub-plan's pass left at the written reach, an `ARRAY[…]` constructor's `unnest`,
        // a `USING` merge of an `integer` and a `bigint`, an aggregate over the enclosing
        // query's column, and that column itself
        (
            "SELECT avg(x) FROM \
             (SELECT (SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) FROM t t2 \
             WHERE t2.k = 2) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[4.0]),
        ),
        (
            "SELECT avg(x) FROM (SELECT COALESCE(\
             (SELECT COALESCE(i, f + (SELECT * FROM unnest(ARRAY[1]))) FROM t t2 WHERE t2.k = 2), \
             0) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[4.0]),
        ),
        (
            "SELECT avg(x) FROM (SELECT (SELECT max(x) FROM unnest(ARRAY[1, 4]) q0(x)) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[4.0]),
        ),
        (
            "SELECT avg(x) FROM \
             (SELECT COALESCE((SELECT max(x) FROM unnest(ARRAY[1, 4]) q0(x)), 0) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[4.0]),
        ),
        (
            "SELECT avg(x) FROM (SELECT (SELECT max(k) FROM t t2 JOIN u8 USING (k)) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[3.0]),
        ),
        (
            "SELECT avg(x) FROM \
             (SELECT COALESCE((SELECT max(k) FROM t t2 JOIN u8 USING (k)), 0) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[3.0]),
        ),
        (
            "SELECT avg(x) FROM (SELECT (SELECT max(t.i + u.i) FROM u) AS x FROM t) q".into(),
            TypeOid::Float8,
            float_avg(&[51.0, 55.0, 52.0]),
        ),
        (
            "SELECT avg(x) FROM (SELECT COALESCE((SELECT max(t.i + u.i) FROM u), 0) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[51.0, 0.0, 55.0, 52.0]),
        ),
        (
            "SELECT avg(x) FROM (SELECT (SELECT t.i) AS x FROM t) q".into(),
            TypeOid::Float8,
            float_avg(&[1.0, 5.0, 2.0]),
        ),
        (
            "SELECT avg(x) FROM (SELECT COALESCE((SELECT t.i), 0) AS x FROM t) q".into(),
            TypeOid::Float8,
            float_avg(&[1.0, 0.0, 5.0, 2.0]),
        ),
        // a wildcard over a constructor's `unnest`, as a branch: neither its own record
        // nor the pass's branch reader types it
        (
            "SELECT avg(x) FROM (SELECT COALESCE(\
             (SELECT * FROM unnest(ARRAY[1, 4]) LIMIT 1), 0) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[1.0]),
        ),
        (
            "SELECT avg(x) FROM (VALUES ((SELECT max(x) FROM unnest(ARRAY[1, 4]) q0(x))), (4)) v(x)"
                .into(),
            TypeOid::Float8,
            float_avg(&[4.0]),
        ),
        // an operand subquery under a fold's branch, its own column a guess
        (
            "SELECT avg(x) FROM \
             (SELECT COALESCE(i, (SELECT * FROM unnest(ARRAY[1])) + 1) AS x FROM t) q"
                .into(),
            TypeOid::Float8,
            float_avg(&[1.0, 2.0, 5.0, 2.0]),
        ),
        (
            "SELECT sum(x) FROM (SELECT COALESCE(\
             (SELECT max(x) FROM unnest(ARRAY[9007199254740993, 1]) q0(x)), 0) AS x FROM t) q"
                .into(),
            TypeOid::Int8,
            Some("36028797018963972".into()),
        ),
        (
            "SELECT avg(x) FROM (SELECT row_number() OVER (ORDER BY k) AS x FROM t) q".into(),
            TypeOid::Float8,
            float_avg(&[1.0, 2.0, 3.0, 4.0]),
        ),
        (
            "SELECT avg(x) FROM unnest(ARRAY[1, 4]) q(x)".into(),
            TypeOid::Float8,
            float_avg(&[1.0, 4.0]),
        ),
        (
            "SELECT avg(k) FROM t JOIN u8 USING (k)".into(),
            TypeOid::Float8,
            float_avg(&[1.0, 3.0]),
        ),
        (
            "SELECT avg(k) FROM (SELECT k FROM t JOIN u8 USING (k)) q".into(),
            TypeOid::Float8,
            float_avg(&[1.0, 3.0]),
        ),
        (
            "SELECT (SELECT avg(v.x + t.i) FROM (VALUES (1), (4)) v(x)) FROM t WHERE k = 1".into(),
            TypeOid::Float8,
            float_avg(&[2.0, 5.0]),
        ),
        (
            "SELECT sum(x) FROM unnest(ARRAY[9007199254740993, 1, 1]) q(x)".into(),
            TypeOid::Int8,
            Some("9007199254740995".into()),
        ),
        (
            "SELECT sum(x) FROM (SELECT row_number() OVER (ORDER BY k) AS x FROM t) q".into(),
            TypeOid::Int8,
            Some("10".into()),
        ),
    ];
    for (sql, declared, value) in uncertain {
        let (types, row) = declared_types_and_first_row(&mut db, sql);
        assert_eq!(types, [*declared], "{sql}");
        assert_eq!(row, std::slice::from_ref(value), "{sql}");
    }
    // a `RETURNING` column, read through the `WITH` query that returns it
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "WITH w AS (INSERT INTO u VALUES (7, 70, 70) RETURNING b) SELECT sum(b), avg(b) FROM w",
    );
    assert_eq!(types, [TypeOid::Int8, TypeOid::Float8]);
    assert_eq!(row, [Some("70".into()), Some("70".into())]);
    // a fold over a guessed column keeps the written reading — the guess's own type, and
    // the `float8` value a fold with no typed branch left unconverted — where a target
    // chosen from the guess would cast `3.5` to `4`
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT COALESCE(q.x * 1, t.b * 1) FROM t, \
         (SELECT COALESCE(i + 0, f + (SELECT * FROM unnest(ARRAY[1]))) AS x FROM t WHERE k = 2) q \
         WHERE t.k = 1",
    );
    assert_eq!(types, [TypeOid::Int4]);
    assert_eq!(row, [Some("3.5".into())]);
}

/// A `double precision` branch decides the target of a fold read at the written reach:
/// where the branches that reach types are numeric and a branch the computed reader
/// types is `double precision`, the set is `double precision`, as 18.6's is whatever
/// numeric types stand beside it — beside a column whose type is right and not certain,
/// beside a branch neither reach types, and with no branch typed at the written reach
/// at all. The computed sibling was cast to the column's declared type — `integer` over
/// `unnest(ARRAY[NULL::int])`, rounding `1.5` to `2`, and `numeric` over a column an
/// exact `avg` gave a `numeric` declaration, where it kept fifteen digits — or the set
/// was left unresolved, declared `integer` over the `double precision` value.
///
/// And the things the rule does not do, each the answer the set gave before: a `real`
/// branch does not decide the target; a branch of an exact type does not join (a
/// guessed `integer` column holding `3.5` cast to a wider exact target answers `4`);
/// and a set with a branch outside the numeric category — by the written reach's
/// reading or, for a branch neither reach types, by the walkers' — is not touched, so
/// the refusal a `greatest` or a `nullif` makes of it is the one it made.
#[test]
fn a_double_precision_branch_decides_the_target_of_a_set_read_at_the_written_reach() {
    let mut db = aggregate_shapes_db();
    db.run("UPDATE t SET f = 0.1234567890123456789 WHERE k = 1");
    let over_unnest = "FROM t, unnest(ARRAY[NULL::int]) u(x) WHERE k = 1";
    let digits = Some("0.12345678901234568".to_string());
    let (types, row) = declared_types_and_first_row(
        &mut db,
        &format!(
            "SELECT COALESCE(u.x, t.f + 0), greatest(u.x, t.f + 0), \
             CASE WHEN u.x IS NULL THEN t.f * 1 ELSE u.x END, nullif(t.f + 0, u.x), \
             COALESCE(u.x, t.f + 0, NULL), COALESCE(u.x, -t.f, sqrt(t.i)) {over_unnest}"
        ),
    );
    assert_eq!(types, [TypeOid::Float8; 6]);
    assert_eq!(
        row,
        [
            digits.clone(),
            digits.clone(),
            digits.clone(),
            digits.clone(),
            digits.clone(),
            Some("-0.12345678901234568".into())
        ]
    );
    let (types, row) = declared_types_and_first_row(
        &mut db,
        &format!("SELECT nullif(u.x, t.f + 0), ARRAY[u.x, t.f + 0] {over_unnest}"),
    );
    assert_eq!(types, [TypeOid::Float8, TypeOid::Float8Array]);
    assert_eq!(row, [None, Some("{NULL,0.12345678901234568}".into())]);
    // no branch typed at the written reach: the sibling is computed over the guessed
    // column, and the set was left unresolved
    let (types, row) = declared_types_and_first_row(
        &mut db,
        &format!(
            "SELECT COALESCE(u.x * 1, t.f + 0), CASE WHEN u.x IS NULL THEN t.f * 1 ELSE -u.x END, \
             greatest(abs(u.x), t.f + 0), nullif(u.x * 1, t.f + 0), ARRAY[u.x + 0, t.f + 0] \
             {over_unnest}"
        ),
    );
    assert_eq!(
        types,
        [
            TypeOid::Float8,
            TypeOid::Float8,
            TypeOid::Float8,
            TypeOid::Float8,
            TypeOid::Float8Array
        ]
    );
    assert_eq!(
        row,
        [
            digits.clone(),
            digits.clone(),
            digits.clone(),
            None,
            Some("{NULL,0.12345678901234568}".into())
        ]
    );
    // a branch neither reach types beside the written column and the float
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT COALESCE(u.x, t.f + 1.375, u.x * 1) FROM t, unnest(ARRAY[NULL::int]) u(x) \
         WHERE k = 2",
    );
    assert_eq!(types, [TypeOid::Float8]);
    assert_eq!(row, [Some("3.875".into())]);
    // a column whose `numeric` declaration is right and not certain: its definition
    // adds a guessed column's aggregate to the exact `avg` of a `VALUES` column
    let moved = "FROM (SELECT avg(v) + max(g.y) AS x \
                 FROM (VALUES (1), (4)) q(v), unnest(ARRAY[0]) g(y) WHERE false) d, t \
                 WHERE t.k = 1";
    let (types, row) = declared_types_and_first_row(
        &mut db,
        &format!(
            "SELECT COALESCE(x, t.f + 0), greatest(x, t.f + 0), nullif(t.f + 0, x), \
             CASE WHEN x IS NULL THEN t.f * 1 ELSE x END {moved}"
        ),
    );
    assert_eq!(types, [TypeOid::Float8; 4]);
    assert_eq!(row, vec![digits; 4]);

    // a `real` branch does not decide the target: the set keeps the written reading
    let (types, row) = declared_types_and_first_row(
        &mut db,
        &format!("SELECT COALESCE(u.x, -((t.f + 2.5)::real)) {over_unnest}"),
    );
    assert_eq!(types, [TypeOid::Int4]);
    assert_eq!(row, [Some("-3".into())]);
    // an exact branch does not join: the guessed `integer` column keeps its `3.5`
    let guessed = "FROM t, (SELECT COALESCE(i + 0, f + (SELECT * FROM unnest(ARRAY[1]))) AS x \
                   FROM t WHERE k = 2) q WHERE t.k = 1";
    for sibling in ["t.b * 1", "t.n * 1"] {
        let (types, row) = declared_types_and_first_row(
            &mut db,
            &format!("SELECT COALESCE(q.x, {sibling}) {guessed}"),
        );
        assert_eq!(types, [TypeOid::Int4], "{sibling}");
        assert_eq!(row, [Some("3.5".into())], "{sibling}");
    }
    // nor does an exact computed branch beside a branch neither reach types
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT COALESCE(u.x, t.n * 1 + 0.25, u.x * 1) FROM t, unnest(ARRAY[NULL::int]) u(x) \
         WHERE k = 2",
    );
    assert_eq!(types, [TypeOid::Int4]);
    assert_eq!(row, [Some("3".into())]);
    // a written reading that is not numeric is the answer it was: the `text`
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT COALESCE(u.tx, t.f + 0), greatest(u.tx, t.f + 0) \
         FROM t, unnest(ARRAY['a']) u(tx) WHERE k = 1",
    );
    assert_eq!(types, [TypeOid::Text, TypeOid::Text]);
    assert_eq!(row, [Some("a".into()), Some("a".into())]);
    // and so is a set with a branch neither reach types that the walkers read as
    // `text`: the `COALESCE` answers it, and the variadics refuse the pair as they did
    let (types, row) = declared_types_and_first_row(
        &mut db,
        "SELECT COALESCE(upper(tx), f + 0) FROM t WHERE k = 1",
    );
    assert_eq!(types, [TypeOid::Text]);
    assert_eq!(row, [Some("A".into())]);
    for (sql, sqlstate) in [
        (
            "SELECT greatest(upper(tx), f + 0) FROM t WHERE k = 1",
            "42804",
        ),
        (
            "SELECT nullif(upper(tx), f + 0) FROM t WHERE k = 1",
            "42883",
        ),
    ] {
        let err = db.try_run(sql).expect_err(sql);
        assert_eq!(err.sqlstate(), sqlstate, "{sql}: {err}");
    }
}

/// The shapes an aggregate's result type is read through: every protocol leg must
/// declare one type for it and send values of that type. Rounds of measured
/// disagreement each found a reader deciding for itself whether an argument's integer
/// type was certain — over a grouped derived column, a fold the coercion pass never
/// unified, a `USING` merged column, a correlated reference — and the descriptors and
/// the value parted. Listed here with the fixtures' own shapes.
const AGGREGATE_SHAPES: &[&str] = &[
    // the plain and fixture shapes
    "SELECT avg(i), sum(i), sum(b), avg(b), avg(s), sum(n), avg(n), avg(f) FROM t",
    "SELECT avg(DISTINCT i), sum(DISTINCT b) FROM t",
    "SELECT COALESCE(sum(b), 0), COALESCE(sum(b), 7), COALESCE(sum(b), 0)::bigint FROM t",
    "SELECT COALESCE(sum(b), 7), COALESCE(avg(i), 7) FROM t WHERE false",
    "SELECT k, avg(i), sum(b) FROM t GROUP BY k HAVING avg(i) > 0 ORDER BY avg(i)",
    "SELECT avg(i) FROM t UNION ALL SELECT avg(b) FROM t",
    "SELECT (SELECT avg(i) FROM t), (SELECT sum(b) FROM t)",
    "SELECT avg(i) % 2, avg(i) * 2, -avg(i), avg(i)::int FROM t",
    "SELECT avg(i + 0), avg(-i), avg(abs(i)), avg(i * 2), avg(i % 2), avg(i / 2) FROM t",
    "SELECT avg(COALESCE(i, 0)), avg(nullif(i, 0)), avg(greatest(i, 0)), avg(length(tx)) FROM t",
    "SELECT avg(i::int8), sum(i::int8), avg(1), sum(9223372036854775807) FROM t",
    // a fold the coercion pass unifies, and one it leaves at the written reading
    "SELECT avg(COALESCE(i + 0, f + 0)), sum(COALESCE(i + 0, f + 0)) FROM t",
    "SELECT avg(CASE WHEN i IS NOT NULL THEN i + 0 ELSE f * 2 END) FROM t",
    "SELECT avg(CASE WHEN i IS NULL THEN length('a') ELSE f + 0 END) FROM t",
    "SELECT avg(COALESCE(abs(i), abs(f))), avg(COALESCE(length('ab'), f + 0)) FROM t",
    "SELECT avg(COALESCE(b + 0, f + 0)) FROM t",
    "SELECT avg(x) FROM (SELECT COALESCE(i + 0, f + 0) AS x FROM t) s",
    "SELECT avg(COALESCE(i + 0, f + (SELECT 1))) FROM t WHERE i IS NOT NULL",
    "SELECT avg(x) FROM (SELECT COALESCE(i + 0, f + (SELECT 1)) AS x FROM t) s",
    "SELECT COALESCE(avg((SELECT 1)), 7) FROM t WHERE false",
    "SELECT COALESCE(avg(CASE WHEN i > 0 THEN i ELSE 0 END), 7) FROM t WHERE false",
    "SELECT COALESCE(avg(nullif(i, 0)), 7), COALESCE(avg(greatest(i, 1)), 7) FROM t WHERE false",
    // a branch the computed reader does not type: `date - date`
    "SELECT avg(COALESCE(d - d2, f + 0)) FROM t",
    "SELECT avg(COALESCE(d - d2, f + 0)) FROM t WHERE i IS NOT NULL",
    "SELECT COALESCE(avg(COALESCE(d - d2, f + 0)), 7) FROM t WHERE false",
    "SELECT avg(COALESCE(abs(d - d2), f + 0)), avg(COALESCE(i + (d - d2), f + 0)) FROM t",
    "SELECT avg(d - d2), sum(d - d2) FROM t",
    // a grouped column, through a derived table, a CTE, a nesting and a scalar subquery
    "SELECT sum(g), avg(g) FROM (SELECT b AS g FROM t GROUP BY b) s",
    "SELECT sum(g) FROM (SELECT b AS g FROM t GROUP BY b) s WHERE false",
    "SELECT avg(g) FROM (SELECT i AS g FROM t GROUP BY i HAVING count(*) > 0) s",
    "SELECT avg(g) FROM (SELECT i AS g FROM t GROUP BY 1) s",
    "SELECT avg(g) FROM (SELECT g FROM (SELECT i AS g FROM t GROUP BY i) s1) s",
    "WITH c AS (SELECT i AS g FROM t GROUP BY i) SELECT avg(g) FROM c",
    "SELECT avg(s.g) FROM (SELECT i AS g FROM t GROUP BY i) s JOIN u ON s.g = u.k",
    "SELECT (SELECT avg(g) FROM (SELECT i AS g FROM t GROUP BY i) s)",
    "SELECT avg(g), sum(c) FROM (SELECT k AS g, count(*) AS c FROM t GROUP BY k) s",
    "SELECT COALESCE(avg(g), 7) FROM (SELECT i AS g FROM t GROUP BY i) s WHERE false",
    // a `USING` merged column
    "SELECT COALESCE(avg(k), 7) FROM t JOIN u USING (k) WHERE false",
    "SELECT COALESCE(avg(k), 7), avg(k), sum(k) FROM t JOIN u USING (k)",
    "SELECT avg(k), sum(k) FROM t FULL JOIN u USING (k)",
    "SELECT avg(k), sum(k) FROM t JOIN u8 USING (k)",
    // relations whose columns an expression defines
    "SELECT avg(x), sum(x) FROM (VALUES (1), (2)) v(x)",
    "SELECT COALESCE(avg(x), 7) FROM (VALUES (1)) v(x) WHERE false",
    "SELECT avg(x), sum(x) FROM (SELECT i AS x FROM t UNION ALL SELECT i FROM u) s",
    "SELECT avg(x), sum(x) FROM unnest(ARRAY[1, 2]) x",
    "WITH RECURSIVE r(x) AS (SELECT 1 UNION ALL SELECT x + 1 FROM r WHERE x < 4) \
     SELECT avg(x), sum(x) FROM r",
    "WITH c(x, y) AS (SELECT i, b FROM t) SELECT avg(x), sum(y) FROM c",
    "SELECT avg(x) FROM (SELECT i + 0 AS x FROM t) s",
    // a relation's column read directly: each value is of the type both legs declare for
    // the column, whichever shape of expression defines it
    "SELECT * FROM (SELECT s + s AS a, s + i AS b, i + b AS c, i / 2 AS d, -s AS e, abs(s) AS g, \
     length(tx) AS h, i::bigint AS j, 1 AS l, 5000000000 AS m, i + 1.5 AS o, i + f AS p FROM t) q",
    "SELECT * FROM (SELECT COALESCE(s, i) AS a, CASE WHEN k > 2 THEN i ELSE b END AS b, \
     greatest(i, b) AS c, nullif(i, b) AS d, COALESCE(i, f + 0) AS e, least(s, i) AS g FROM t) q",
    "SELECT * FROM (SELECT count(*) AS a, sum(s) AS b, sum(i) AS c, sum(b) AS d, avg(i) AS e, \
     min(s) AS g, max(b) AS h, round(avg(i)) AS j, sum(i) + 1 AS l FROM t GROUP BY k % 2) q",
    "SELECT * FROM (VALUES (1, 1::smallint, 1.5, '7', NULL), (5000000000, 2, 2, 8, NULL)) \
     v(a, b, c, d, e)",
    "SELECT * FROM (SELECT s AS x FROM t UNION ALL SELECT b FROM t) q",
    "SELECT * FROM (SELECT i AS x FROM t UNION SELECT n FROM t) q",
    "SELECT * FROM (SELECT i AS x FROM t INTERSECT SELECT b FROM t) q",
    "SELECT * FROM (SELECT b AS x FROM t UNION ALL SELECT NULL) q",
    "WITH RECURSIVE r(x) AS (SELECT 1::bigint UNION ALL SELECT (x + 1)::int FROM r WHERE x < 3) \
     SELECT x FROM r",
    "SELECT x, y FROM unnest('{1,2}'::int[], '{5000000000}'::bigint[]) q(x, y)",
    "SELECT * FROM (SELECT * FROM t FULL JOIN u8 USING (k)) q",
    "SELECT avg(k), sum(k) FROM (SELECT * FROM t FULL JOIN u8 USING (k)) q",
    // and aggregated, over values an exact accumulation and a `float8` one tell apart
    "SELECT avg(x), sum(x), round(avg(x)) FROM (VALUES (9007199254740993), (1), (1)) v(x)",
    "SELECT avg(x), sum(x), round(avg(x)) FROM (SELECT b + 0 AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (SELECT max(b) AS x FROM t GROUP BY k) q",
    "SELECT avg(x), sum(x) FROM (SELECT count(*) AS x FROM t GROUP BY k % 2) q",
    "SELECT avg(x), sum(x) FROM (SELECT b AS x FROM t UNION ALL SELECT i FROM u) q",
    "SELECT avg(x), sum(x) FROM (SELECT b AS x FROM t UNION ALL SELECT NULL) q",
    "SELECT avg(x), sum(x) FROM (SELECT x * 1 AS x FROM (SELECT b + 0 AS x FROM t) q0) q",
    "WITH RECURSIVE r(x) AS (SELECT 9007199254740993 UNION ALL \
     SELECT x - 9007199254740992 FROM r WHERE x > 2) SELECT avg(x), sum(x) FROM r",
    "SELECT avg(x), sum(x) FROM unnest('{9007199254740993,1,1}'::bigint[]) q(x)",
    "SELECT avg(k), sum(k) FROM (SELECT k FROM t JOIN u USING (k)) q",
    "SELECT k, COALESCE(v.x, t.f + 0), greatest(v.x, t.n * 1) FROM t, (VALUES (NULL::int)) v(x) \
     ORDER BY k",
    // a scalar subquery whose own column is certain, defining a column as the entry, an
    // operand and a branch of each fold, read directly and aggregated
    "SELECT * FROM (SELECT (SELECT max(b) FROM t t2 WHERE t2.k <= t.k) AS a, (SELECT 1) + i AS b, \
     COALESCE((SELECT max(i) FROM u), 0) AS c, ARRAY[(SELECT 1), 2] AS d, (SELECT avg(i) FROM u) AS e, \
     CASE WHEN k > 2 THEN (SELECT b FROM t t2 WHERE t2.k = t.k) ELSE i END AS g, \
     greatest((SELECT 5000000000), i) AS h, nullif((SELECT i FROM t t2 WHERE t2.k = t.k), 5) AS j, \
     COALESCE((SELECT avg(i) FROM u WHERE false), f + 0) AS l, -(SELECT max(b) FROM t) AS m FROM t) q",
    "SELECT avg(x), sum(x), round(avg(x)) FROM \
     (SELECT COALESCE((SELECT b FROM t t2 WHERE t2.k = t.k), 1) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (SELECT (SELECT max(b) FROM t t2 WHERE t2.k <= t.k) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (SELECT (SELECT b FROM t t2 WHERE t2.k = t.k) + 0 AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (VALUES ((SELECT max(b) FROM t)), (1), (1)) v(x)",
    "SELECT avg(x), sum(x) FROM (SELECT (SELECT 1) AS x FROM t) q",
    "SELECT k, COALESCE(x, t.f + 0), greatest(x, t.f + 0) \
     FROM (SELECT (SELECT avg(i) FROM u WHERE i > 99) AS x) q, t ORDER BY k",
    // and one whose column is a guess, as a bare branch: the cast the pass writes over
    // the branch is not a certainty
    "SELECT k, COALESCE((SELECT max(x) FROM unnest(ARRAY[1, 2.5]) q0(x)), 0), \
     COALESCE((SELECT max(k) FROM t t2 FULL JOIN u8 USING (k)), 0), \
     CASE WHEN k > 0 THEN (SELECT d - d2 FROM t t2 WHERE t2.k = t.k) ELSE 0 END, \
     greatest((SELECT COALESCE(i, f + (SELECT 1)) FROM t t2 WHERE t2.k = 2), 0) FROM t ORDER BY k",
    "SELECT avg(x), sum(x) FROM \
     (SELECT COALESCE((SELECT max(x) FROM unnest(ARRAY[1, 2.5]) q0(x)), 0) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM \
     (SELECT COALESCE((SELECT max(k) FROM t t2 FULL JOIN u8 USING (k)), 0) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (SELECT COALESCE((SELECT max(t.i + u.i) FROM u), 0) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (SELECT COALESCE((SELECT t.i), 0) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM \
     (SELECT COALESCE((SELECT COALESCE(i, f + (SELECT 1)) FROM t t2 WHERE t2.k = 2), 0) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (SELECT COALESCE(i, (SELECT 1) + 1) AS x FROM t) q",
    // a subquery over a column that holds another type's value under its declaration (a
    // fold left bare, `EDGES.md` row 152): called certain, an exact `avg` would refuse
    // the `numeric` it reads under `integer`
    "SELECT avg(x) FROM (SELECT (SELECT x FROM (SELECT COALESCE(i, \
     (SELECT * FROM unnest(ARRAY[1.5::numeric]))) AS x FROM t t2 WHERE t2.k = 2) q0) AS x FROM t) q",
    // a set read at the written reach, a float branch in its target
    "SELECT k, COALESCE(u.x, t.f + 0), greatest(u.x, t.f + 0), nullif(u.x, t.f + 0), \
     nullif(t.f + 0, u.x), ARRAY[u.x, t.f + 0] FROM t, unnest(ARRAY[NULL::int, 7]) u(x) \
     WHERE t.f IS NOT NULL ORDER BY k",
    "SELECT COALESCE(x, t.f + 0), CASE WHEN x IS NULL THEN t.f * 1 ELSE x END FROM \
     (SELECT avg(v) + (max(d) - max(d2)) AS x FROM (VALUES (1), (4)) q(v), t t2) d, t",
    // a fold over an operand the computed reach types by a subquery's certain column or
    // as a `date` less a `date`, beside a `double precision` branch and beside exact ones
    "SELECT k, COALESCE(-(SELECT avg(x) FROM (VALUES (1), (4)) v(x) WHERE x > t.k * 2), t.f + 0), \
     CASE WHEN k = 1 THEN t.f + 0 ELSE -(SELECT avg(x) FROM (VALUES (1), (4)) v(x)) END, \
     greatest((SELECT avg(i) FROM u) + 1, t.f + 0), nullif(t.f + 0, abs((SELECT sum(b) FROM u))), \
     ARRAY[(SELECT avg(x) FROM (VALUES (1), (4)) v(x)) * 2, t.f * 1] FROM t ORDER BY k",
    "SELECT sum(y), avg(y) FROM (SELECT COALESCE(-(SELECT avg(x) FROM (VALUES (1), (4)) v(x) \
     WHERE x > t.k * 2), t.f + 0) AS y FROM t WHERE k <> 2) q",
    "SELECT k, COALESCE(i, f + (SELECT 1)), COALESCE(i + 0, b + (SELECT 0)), \
     COALESCE(-(SELECT avg(x) FROM (VALUES (1), (4)) v(x)), b), \
     greatest(i, (SELECT max(b) FROM u) - 1), ARRAY[i + 0, f + (SELECT 1)] FROM t ORDER BY k",
    "SELECT COALESCE(avg(v.x) + (max(d) - max(d2)), max(f) + 0), greatest(sum(v.x), max(f) * 1) \
     FROM (SELECT count(*) AS x FROM t GROUP BY k) v, t",
    "SELECT k, COALESCE(d - d2, f + 0), COALESCE((d - d2) * 2, n * 1), greatest(d - d2, i + 0), \
     CASE WHEN k = 1 THEN f + 0 ELSE d - d2 END, ARRAY[d - d2, f * 1] FROM t ORDER BY k",
    "SELECT avg(d - d2), sum(d - d2), avg(COALESCE(d - d2, i)), max(d - d2), avg((d - d2) * 2) FROM t",
    // an operand of unknown type under arithmetic, which no reader types
    "SELECT avg(i + NULL), sum(b * NULL), COALESCE(avg(i), max(f) + NULL), \
     greatest(avg(i) + NULL, min(f) * 1) FROM t",
    // a `double precision` branch beside a branch neither reach types
    "SELECT k, COALESCE(u.x * 1, t.f + 0), greatest(abs(u.x), t.f + 0), \
     COALESCE(u.x, t.f + 0, u.x * 1), ARRAY[u.x + 0, t.f + 0] \
     FROM t, unnest(ARRAY[NULL::int, 7]) u(x) WHERE t.f IS NOT NULL ORDER BY k",
    // a wildcard subquery over a cast array's `unnest`, a fold's branch
    "SELECT COALESCE((SELECT * FROM unnest('{1.5}'::float8[])), 2.5), \
     greatest((SELECT * FROM unnest('{1}'::int[])), 5000000000), \
     (SELECT avg(x) FROM (SELECT COALESCE((SELECT * FROM unnest('{1,4}'::int[]) LIMIT 1), i) AS x \
     FROM t) q)",
    // and a fold with an operand subquery whose column is a guess, read where it agrees
    "SELECT avg(COALESCE(i + 0, f + (SELECT * FROM unnest(ARRAY[1])))) FROM t WHERE i IS NOT NULL",
    "SELECT avg(x), sum(x) FROM \
     (SELECT COALESCE(i, (SELECT * FROM unnest(ARRAY[1])) + 1) AS x FROM t) q",
    // a relation's column whose definition the coercion pass did not type as written
    "SELECT avg(x), sum(x) FROM (SELECT d - d2 AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (SELECT (SELECT d - d2 FROM t t2 WHERE t2.k = t.k) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (SELECT row_number() OVER (ORDER BY k) AS x FROM t) q",
    "SELECT avg(x), sum(x) FROM (VALUES (COALESCE(1, 2.5 + (SELECT 1))), (4)) v(x)",
    "SELECT avg(x), sum(x) FROM unnest(ARRAY[9007199254740993, 1, 1]) q(x)",
    "SELECT avg(k), sum(k) FROM (SELECT k FROM t JOIN u8 USING (k)) q",
    "SELECT k, (SELECT avg(v.x + t.i) FROM (VALUES (1), (4)) v(x)) FROM t ORDER BY k",
    // correlated references, which the coercion pass decides on its re-run
    "SELECT k, (SELECT avg(t.i + u.i) FROM u) FROM t ORDER BY k",
    "SELECT k, (SELECT COALESCE(avg(t.i + u.i), 7) FROM u WHERE false) FROM t ORDER BY k",
    "SELECT g, (SELECT avg(s.g + u.i) FROM u) FROM (SELECT i AS g FROM t) s ORDER BY g",
    "SELECT g, (SELECT avg(s.g + u.i) FROM u) FROM (SELECT i AS g FROM t GROUP BY i) s ORDER BY g",
    "SELECT k, (SELECT avg(COALESCE(u.i, u.b + (SELECT 0))) FROM u WHERE u.k = t.k) FROM t ORDER BY k",
    "SELECT k, (SELECT avg(t.i + COALESCE(u.i, u.b + (SELECT 0))) FROM u) FROM t ORDER BY k",
    // a correlated aggregate read by its enclosing level's fold, branch or set operation
    "SELECT k, COALESCE((SELECT avg(t.b + u.b) FROM u), 7) FROM t ORDER BY k",
    "SELECT k, CASE WHEN k > 0 THEN (SELECT avg(t.b + u.b) FROM u) ELSE 0 END FROM t ORDER BY k",
    "SELECT k, greatest((SELECT avg(t.b + u.b) FROM u), 0), nullif((SELECT avg(t.i + u.i) FROM u), -1) \
     FROM t ORDER BY k",
    "SELECT k, ARRAY[(SELECT avg(t.i + u.i) FROM u), 7] FROM t ORDER BY k",
    "SELECT k, COALESCE((SELECT sum(t.i + u.b) FROM u), 7), COALESCE((SELECT sum(t.i + u.i) FROM u), 7) \
     FROM t ORDER BY k",
    "SELECT k, (SELECT COALESCE(avg(t.b + u.b), 7) FROM u) FROM t ORDER BY k",
    "SELECT (SELECT avg(t.b + u.b) FROM u) FROM t UNION ALL SELECT 1.5",
];

/// Each fold, branch or set operation over a correlated aggregate beside the bare
/// aggregate it wraps: where the bare one answers a value the wrapper passes through, the
/// two answer it alike, type and digits. `uw` holds 2^53, 1 and 1, which an exact
/// accumulation and a `float8` one tell apart, so an aggregate the enclosing level read as
/// one type and its re-run decided as another — answering the exact quotient cast to
/// `double precision`, a third value — parts the pair.
const WRAPPED_AGGREGATES: &[(&str, &str)] = &[
    (
        "SELECT k, COALESCE((SELECT avg(tw.b + uw.b) FROM uw), 7) FROM tw",
        "SELECT k, (SELECT avg(tw.b + uw.b) FROM uw) FROM tw",
    ),
    (
        "SELECT k, CASE WHEN k > 0 THEN (SELECT avg(tw.b + uw.b) FROM uw) ELSE 0 END FROM tw",
        "SELECT k, (SELECT avg(tw.b + uw.b) FROM uw) FROM tw",
    ),
    (
        "SELECT k, greatest((SELECT avg(tw.b + uw.b) FROM uw), 0) FROM tw",
        "SELECT k, (SELECT avg(tw.b + uw.b) FROM uw) FROM tw",
    ),
    (
        "SELECT k, nullif((SELECT avg(tw.b + uw.b) FROM uw), -1) FROM tw",
        "SELECT k, (SELECT avg(tw.b + uw.b) FROM uw) FROM tw",
    ),
    (
        "SELECT k, COALESCE((SELECT sum(tw.i + uw.b) FROM uw), 7) FROM tw",
        "SELECT k, (SELECT sum(tw.i + uw.b) FROM uw) FROM tw",
    ),
    (
        "SELECT k, (SELECT avg(tw.b + uw.b) FROM uw) FROM tw UNION ALL SELECT 0, 1.5",
        "SELECT k, (SELECT avg(tw.b + uw.b) FROM uw) FROM tw",
    ),
    (
        "SELECT k, (SELECT COALESCE(avg(tw.b + uw.b), 7) FROM uw) FROM tw",
        "SELECT k, (SELECT avg(tw.b + uw.b) FROM uw) FROM tw",
    ),
];

/// Every aggregate shape declares one type on both protocol legs, and every value it
/// sends is of that type: the extended protocol's `Describe` (the planner's
/// `Plan::result_shape`), the simple protocol's `RowDescription` (the executor's
/// descriptor walk) and each returned value's own type agree, column by column. A
/// differential fixture compares each leg's declared type with 18.6's; this compares
/// the legs with the bytes, which is what a binary client decodes by.
///
/// The corpus reads a derived relation's columns directly as well as through an
/// aggregate: a column the coercion pass calls certain is one whose values are of the
/// type declared for it, and an exact accumulator over one that held another type's value
/// would refuse the statement here.
#[test]
fn every_aggregate_shape_declares_its_values_type_on_both_legs() {
    let mut db = aggregate_shapes_db();
    for sql in AGGREGATE_SHAPES
        .iter()
        .chain(WRAPPED_AGGREGATES.iter().map(|(w, _)| w))
    {
        let mut plans = parse(sql, &db.schema).unwrap_or_else(|e| panic!("{sql}: {e}"));
        let describe: Vec<TypeOid> = plans[0]
            .result_shape(&db.schema)
            .columns()
            .iter()
            .map(|c| c.type_oid)
            .collect();
        drop(plans.remove(0));
        let result = db
            .try_run(sql)
            .unwrap_or_else(|e| panic!("{sql}: every shape answers, got {e}"));
        let simple: Vec<TypeOid> = result.columns().iter().map(|c| c.type_oid).collect();
        assert_eq!(
            describe, simple,
            "{sql}: Describe against the simple descriptor"
        );
        for row in &result.rows {
            for (value, declared) in row.iter().zip(&simple) {
                if let Some(actual) = value.type_oid() {
                    assert_eq!(actual, *declared, "{sql}: {value:?} under {declared:?}");
                }
            }
        }
    }
    for (wrapped, bare) in WRAPPED_AGGREGATES {
        let wrapped_rows = db.run(wrapped).rows;
        let bare_rows = db.run(bare).rows;
        for bare_row in &bare_rows {
            let Some(wrapped_row) = wrapped_rows.iter().find(|r| r[0] == bare_row[0]) else {
                panic!("{wrapped}: no row for {:?}", bare_row[0]);
            };
            if !bare_row[1].is_null() {
                assert_eq!(wrapped_row[1], bare_row[1], "{wrapped} against {bare}");
            }
        }
    }
}

/// A grouped query's column is described on the extended protocol's `Describe` as the
/// simple protocol describes it: its base table, its attribute number and its modifier
/// pass through, as a projected column's do — directly, beside an aggregate, under a
/// `HAVING`, and through a derived table or a `WITH` query over the grouped query. The
/// column's integer type is certain to the coercion pass through such a relation too, so
/// `avg` over it through a derived table is `numeric` on both legs.
#[test]
fn a_grouped_column_is_described_alike_on_both_legs() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (v VARCHAR(5), n NUMERIC(6,2), k INT)");
    db.run("INSERT INTO t VALUES ('a', 1.5, 1), ('b', 2.5, 2)");
    for sql in [
        "SELECT v, n, k FROM t GROUP BY v, n, k",
        "SELECT v AS x, count(*) FROM t GROUP BY 1",
        "SELECT n FROM t GROUP BY n HAVING n > 0",
        "SELECT * FROM (SELECT v, k FROM t GROUP BY v, k) d",
        "WITH c AS (SELECT v, n FROM t GROUP BY v, n) SELECT * FROM c",
    ] {
        let plans = parse(sql, &db.schema).expect("parses");
        let describe = plans[0].result_shape(&db.schema).columns().to_vec();
        let simple = db.run(sql).columns().to_vec();
        let meta = |c: &pglet_sql::types::ColumnDesc| {
            (c.type_oid, c.table_oid, c.column_id, c.type_modifier)
        };
        assert_eq!(
            describe.iter().map(meta).collect::<Vec<_>>(),
            simple.iter().map(meta).collect::<Vec<_>>(),
            "{sql}"
        );
        assert!(simple[0].table_oid != 0, "{sql}: a base column");
    }
    let result = db.run("SELECT avg(g) FROM (SELECT k AS g FROM t GROUP BY k) s");
    assert_eq!(result.columns()[0].type_oid, TypeOid::Numeric);
}

/// The first statement of a correlated aggregate's argument decides it. An argument over an
/// enclosing query's column cannot be typed by the sub-plan's own run, so it is decided
/// there as not certain — `avg` is `double precision` and `sum(bigint)` `bigint`, the
/// answers an argument of unknown type gets — and the correlated re-run keeps that verdict
/// rather than read the argument the first run already resolved. The enclosing level reads
/// the same verdict when it chooses its own branch types, so a fold, a `CASE`, a
/// `greatest`, an `ARRAY[…]` or a set operation over the subquery answers what the bare
/// subquery answers. Over values an exact accumulation and a `float8` one tell apart, the
/// value is the `float8` accumulation's: an exact quotient cast to `double precision`
/// afterwards would be a third answer, neither 18.6's `numeric` nor this one.
#[test]
fn a_correlated_aggregate_is_decided_on_its_first_run() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (k INT, i INT, b BIGINT, tx TEXT)");
    db.run("INSERT INTO t VALUES (1, 1, 0, 'a')");
    db.run("CREATE TABLE u (k INT, i INT, b BIGINT, f FLOAT8)");
    // `u.i` is never NULL, so no row evaluates a subquery branch — a scalar subquery
    // inside an aggregate's argument is refused `0A000` when a row reaches it
    db.run(
        "INSERT INTO u VALUES (1, 2, 9007199254740992, 2.5), (2, 3, 1, NULL), \
         (3, 4, 1, NULL)",
    );
    // the `float8` accumulation of 2^53, 1 and 1, in input order
    let float_avg = Value::Float8((9_007_199_254_740_992_f64 + 1.0 + 1.0) / 3.0);
    for sql in [
        "SELECT (SELECT avg(t.b + u.b) FROM u) FROM t",
        "SELECT COALESCE((SELECT avg(t.b + u.b) FROM u), 7) FROM t",
        "SELECT CASE WHEN k = 1 THEN (SELECT avg(t.b + u.b) FROM u) ELSE 0 END FROM t",
        "SELECT greatest((SELECT avg(t.b + u.b) FROM u), 0) FROM t",
        "SELECT (SELECT COALESCE(avg(t.b + u.b), 7) FROM u) FROM t",
    ] {
        let plans = parse(sql, &db.schema).expect("parses");
        assert_eq!(
            plans[0].result_shape(&db.schema).columns()[0].type_oid,
            TypeOid::Float8,
            "{sql}: Describe"
        );
        let result = db.run(sql);
        assert_eq!(
            result.columns()[0].type_oid,
            TypeOid::Float8,
            "{sql}: simple"
        );
        assert_eq!(result.rows[0][0], float_avg, "{sql}");
    }
    for (sql, want) in [
        (
            "SELECT (SELECT avg(t.i + u.i) FROM u) FROM t",
            TypeOid::Float8,
        ),
        (
            "SELECT (SELECT avg(t.i + u.i) FROM u WHERE false) FROM t",
            TypeOid::Float8,
        ),
        (
            "SELECT (SELECT sum(t.b + u.b) FROM u) FROM t",
            TypeOid::Int8,
        ),
        (
            "SELECT COALESCE((SELECT sum(t.b + u.b) FROM u), 7) FROM t",
            TypeOid::Int8,
        ),
        // decided on the first run, all its columns local, over an operand subquery
        // whose column is a guess: kept on the re-run, which reads the `float8` branch
        // already cast to `integer`
        (
            "SELECT (SELECT avg(COALESCE(u.i, u.f + (SELECT * FROM unnest(ARRAY[0])))) FROM u \
             WHERE u.k = t.k) FROM t",
            TypeOid::Float8,
        ),
        // an argument whose type does not turn on the enclosing column's is certain on the
        // first run, and the enclosing fold reads that verdict too
        (
            "SELECT (SELECT avg(length(t.tx) + u.i) FROM u) FROM t",
            TypeOid::Numeric,
        ),
        (
            "SELECT COALESCE((SELECT avg(length(t.tx) + u.i) FROM u), 7) FROM t",
            TypeOid::Numeric,
        ),
    ] {
        let plans = parse(sql, &db.schema).expect("parses");
        assert_eq!(
            plans[0].result_shape(&db.schema).columns()[0].type_oid,
            want,
            "{sql}: Describe"
        );
        let result = db.run(sql);
        assert_eq!(result.columns()[0].type_oid, want, "{sql}: simple");
        for row in &result.rows {
            if let Some(actual) = row[0].type_oid() {
                assert_eq!(actual, want, "{sql}: {:?}", row[0]);
            }
        }
    }
}

/// An `ARRAY[…]` branch some of whose elements the branch reader cannot type keeps its
/// set at the written reading, and the gate judges it there: beside `ARRAY[1]`, an array
/// with an element over a scalar subquery whose column is a guess is the `42846` it was
/// before computed branches were typed, rather than an answer declared `integer[]` over
/// `float8` elements (18.6 answers `double precision[]`; refusing is the loud side of
/// that). Over a subquery whose column is certain the element is typed, and the set is
/// 18.6's.
#[test]
fn a_partly_typed_array_branch_is_judged_at_the_written_reading() {
    let mut db = TestDb::new();
    db.run("CREATE TABLE t (i INT, f FLOAT8)");
    db.run("INSERT INTO t VALUES (1, 1.5)");
    let err = db
        .try_run(
            "SELECT COALESCE(ARRAY[i + 0, f + (SELECT * FROM unnest(ARRAY[1]))], ARRAY[1]) FROM t",
        )
        .expect_err("the written reading has no common type");
    assert_eq!(err.sqlstate(), "42846", "{err}");
    for sql in [
        "SELECT COALESCE(ARRAY[i + 0, f + 0], ARRAY[1]) FROM t",
        "SELECT COALESCE(ARRAY[i + 0, f + (SELECT 1)], ARRAY[1]) FROM t",
    ] {
        let typed = db.run(sql);
        assert_eq!(typed.columns()[0].type_oid, TypeOid::Float8Array, "{sql}");
    }
}
