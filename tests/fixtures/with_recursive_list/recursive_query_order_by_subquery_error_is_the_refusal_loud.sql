-- An error analysing a subquery inside the clause over a recursive query is not the
-- verdict: 18.6 refuses the `ORDER BY` before it analyses anything in it.
-- @setup
CREATE TABLE t (a INT);
-- @expect-error: ORDER BY in a recursive query (0A000)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 ORDER BY (SELECT nosuch FROM t))
SELECT * FROM r;
