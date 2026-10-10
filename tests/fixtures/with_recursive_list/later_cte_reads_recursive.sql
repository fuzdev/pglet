-- A CTE listed after a genuinely recursive one reads its rows as it would any
-- relation's. A reference to the earlier recursive CTE is not a self-reference of the
-- CTE being parsed: counting it as one refused this `42P19`.
-- @test
WITH RECURSIVE r(n) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3),
  q AS (SELECT * FROM r)
SELECT n FROM q ORDER BY n;
