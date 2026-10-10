-- A subquery in a CHECK constraint is forbidden — Postgres raises 0A000 (`cannot
-- use subquery in check constraint`). pglet must match: the CHECK expr is
-- postcard-persisted, so a plan-bearing `Expr` (a subquery — which could itself
-- carry a window) must never reach it. Rejected at DDL time, not at INSERT.
-- @expect-error: cannot use subquery in check constraint (0A000)
-- @test
CREATE TABLE sq (a INT CHECK (a > (SELECT 1)));
