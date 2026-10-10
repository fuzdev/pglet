-- A window function in a partial-index predicate is forbidden — Postgres raises
-- 42P20 (`window functions are not allowed in index predicates`). pglet must match
-- the SQLSTATE: the predicate `Expr` is postcard-persisted, so an `Expr::Window`
-- must never reach it.
-- @expect-error: window functions are not allowed in index predicates (42P20)
-- @setup
CREATE TABLE iw (a INT);
-- @test
CREATE INDEX iw_ix ON iw (a) WHERE (row_number() OVER () > 0);
