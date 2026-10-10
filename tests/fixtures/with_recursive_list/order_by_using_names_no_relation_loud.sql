-- The names after `ORDER BY … USING <,` are sort keys, not relations: `q` is no
-- dependency of `r`, so there is no cycle, and the `ORDER BY` refusal stands.
-- @expect-error: ORDER BY in a recursive query (0A000)
-- @expect-message
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3 ORDER BY n USING <, q),
  q AS (SELECT * FROM r)
SELECT * FROM q;
