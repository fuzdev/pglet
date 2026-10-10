-- @setup
CREATE TABLE sjk1 (a INT);
INSERT INTO sjk1 VALUES (1), (2);
CREATE TABLE sjk2 (a INT);

-- A bare `ORDER BY` key inside an expression subquery is judged by the plan-time
-- binding gate or by nobody: the outer `WHERE` matches no row, so the executor never
-- evaluates the key. `resolve_correlated_subplans` used to hand its `Sort` keys a scope
-- that declined to judge **every** bare name (the flag Postgres reserves for a key that
-- names an *output* column), and this answered `(0 rows)`. The write polarity of the
-- same statement is `crates/pglet/tests/scope_relation_identity.rs` —
-- `@expect-error` compares SQLSTATEs and is green for a refusal raised after the writes.
-- @expect-error: a bare sort key inside a subquery that binds to nothing is 42703
-- @test
SELECT a FROM sjk1 WHERE 1 = 0 AND a NOT IN (SELECT a FROM sjk2 ORDER BY bogus);
