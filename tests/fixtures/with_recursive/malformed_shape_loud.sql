-- A recursive CTE body must be `non-recursive-term UNION [ALL] recursive-term`; a
-- different set-op combinator between the terms is not a valid recursive query.
-- @test
-- @expect-error: combinator other than UNION between the terms (42P19)
WITH RECURSIVE t(n) AS (
  SELECT 1
  INTERSECT
  SELECT n + 1 FROM t WHERE n < 5
)
SELECT n FROM t;
