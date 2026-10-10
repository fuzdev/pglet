-- A window function in a CHECK constraint is forbidden — Postgres raises 42P20
-- (`window functions are not allowed in check constraints`). pglet must match the
-- SQLSTATE (and reject at DDL time), not silently accept it: the CHECK expr is
-- postcard-persisted in the catalog, so an `Expr::Window` must never reach it.
-- @expect-error: window functions are not allowed in check constraints (42P20)
-- @test
CREATE TABLE wc (a INT CHECK (row_number() OVER () > 0));
