-- A malformed clause over a recursive query whose seed reads a CTE declared after it: the
-- raw parse's `42601` still outranks every verdict.
-- @expect-error: syntax error (42601)
-- @test
WITH RECURSIVE r(n) AS (SELECT k FROM base UNION ALL SELECT n + 1 FROM r WHERE n < 3 LIMIT 1 garbage),
  base(k) AS (SELECT 1)
SELECT * FROM r;
