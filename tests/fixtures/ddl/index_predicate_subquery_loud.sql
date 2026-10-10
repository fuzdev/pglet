-- A subquery in a partial-index predicate is forbidden — Postgres raises 0A000
-- (`cannot use subquery in index predicate`). pglet must match the SQLSTATE: the
-- predicate `Expr` is postcard-persisted.
-- @expect-error: cannot use subquery in index predicate (0A000)
-- @setup
CREATE TABLE isq (a INT);
-- @test
CREATE INDEX isq_ix ON isq (a) WHERE (a > (SELECT 1));
