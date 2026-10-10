-- A window function reachable only INSIDE a subquery in a CHECK constraint is
-- rejected as the SUBQUERY (0A000 `cannot use subquery in check constraint`) —
-- Postgres never descends to the window, so the SQLSTATE is 0A000, not 42P20. This
-- closes the persistence hole: previously pglet accepted the CREATE (persisting an
-- `Expr::Window` in the catalog) and failed only at INSERT with a non-PG error.
-- @expect-error: cannot use subquery in check constraint (0A000) — window never reached
-- @test
CREATE TABLE wsq (a INT CHECK (a > (SELECT row_number() OVER ())));
