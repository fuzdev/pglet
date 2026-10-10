-- `WITH RECURSIVE` whose CTE never self-references degrades to an ordinary CTE
-- (Postgres does the same silently) — no error, just a plain single-row result.
-- @test
WITH RECURSIVE t AS (
  SELECT 42 AS answer
)
SELECT answer FROM t;
