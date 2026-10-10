-- The `ALTER TABLE … ALTER COLUMN … SET DEFAULT` route to a persisted DEFAULT, which
-- `default_exists_subquery` (the `CREATE TABLE` column spelling) does not reach.
-- What it pins is 18.6's SQLSTATE on this route: `0A000 cannot use subquery in
-- DEFAULT expression`. Not which gate answers — the harness compares SQLSTATE only,
-- and `executor::ddl`'s `check_default_expr_nodes` is not the sole `0A000` on this
-- path; `planner::expr`'s `plan_is_not_persistable` is the gate-identity guard.
-- @expect-error: cannot use subquery in DEFAULT expression (0A000)
-- @setup
CREATE TABLE sdsq (a INT);
-- @test
ALTER TABLE sdsq ALTER COLUMN a SET DEFAULT (SELECT 1);
