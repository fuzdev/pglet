-- An aggregate in an index key expression is forbidden — Postgres raises 42803
-- (`aggregate functions are not allowed in index expressions`). pglet must match the
-- SQLSTATE.
-- @expect-error: aggregate functions are not allowed in index expressions (42803)
-- @setup
CREATE TABLE eag (a INT);
-- @test
CREATE INDEX eag_ix ON eag ((sum(a)));
