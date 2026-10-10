-- The `ALTER TABLE … ADD CONSTRAINT` route to a persisted CHECK, which
-- `check_subquery_loud` (the `CREATE TABLE` column-constraint spelling) does not
-- reach. What this pins is 18.6's SQLSTATE on this route — `0A000`, message `cannot
-- use subquery in check constraint`. It does not pin *which* gate answers: the
-- harness compares SQLSTATE, and a second refusal on this path (the constraint cook,
-- `0A000 not implemented: scalar subquery requires execution context`) shares the
-- code, so this stays green with `reject_persisted_expr_constructs` deleted —
-- measured. The gate-identity guard is `planner::expr`'s `plan_is_not_persistable`.
-- @expect-error: cannot use subquery in check constraint (0A000)
-- @setup
CREATE TABLE acsq (a INT);
-- @test
ALTER TABLE acsq ADD CONSTRAINT acsq_ck CHECK (a > (SELECT 1));
