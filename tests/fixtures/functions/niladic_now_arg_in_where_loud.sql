-- The class is engine-wide rather than projection-specific: `build_call` is the one
-- site a function call is built at, so a predicate position takes the same verdict.
-- The relation is seeded and non-empty so nothing about the row count can stand in for
-- the refusal.
-- @setup
CREATE TABLE nb_where (a INT);
INSERT INTO nb_where VALUES (1);
-- @test
-- @expect-error: function now(integer) does not exist
SELECT a FROM nb_where WHERE now(1) > now();
