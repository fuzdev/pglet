-- `now()` is stable, so 18.6 does not fold `now() IS NULL` and the `CASE` arm's `sum`
-- survives to run. Computing an aggregate only where the `CASE` reads it would answer
-- `0`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @expect-error: division by zero
-- @test
SELECT CASE WHEN now() IS NULL THEN sum(1/b) ELSE 0 END FROM u;
