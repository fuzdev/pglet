-- A cycle through a non-recursive middle CTE is mutual recursion too.
-- @expect-error: mutual recursion through a middle CTE (0A000)
-- @expect-message
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n FROM q),
  q AS (SELECT * FROM a)
SELECT * FROM a;
