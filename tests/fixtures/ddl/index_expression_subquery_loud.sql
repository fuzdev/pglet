-- A subquery in an index key expression is forbidden — Postgres raises 0A000
-- (`cannot use subquery in index expression`). pglet must match the SQLSTATE: the
-- index-expression `Expr` is postcard-persisted.
-- @expect-error: cannot use subquery in index expression (0A000)
-- @setup
CREATE TABLE esq (a INT);
-- @test
CREATE INDEX esq_ix ON esq ((a + (SELECT 1)));
