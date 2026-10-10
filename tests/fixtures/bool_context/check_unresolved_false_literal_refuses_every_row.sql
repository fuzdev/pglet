-- `CHECK ('f')` is a constraint stock Postgres accepts at DDL and that then
-- refuses every row. pglet admitted them: `enforce_checks` violated only on an
-- exact `Bool(false)`, so a non-boolean predicate passed — the opposite polarity
-- from the filters, and the same resolver answers both.
-- @setup
CREATE TABLE bcck (a INT CHECK ('f'));
-- @expect-error: check constraint violated for every row
-- @test
INSERT INTO bcck VALUES (9);
