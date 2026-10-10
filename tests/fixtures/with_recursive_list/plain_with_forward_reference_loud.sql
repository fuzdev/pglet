-- Control: without `RECURSIVE` a CTE sees only the ones declared before it, so a
-- forward reference names a relation, and here there is none.
-- @expect-error: a plain WITH forward reference names no relation (42P01)
-- @test
WITH q AS (SELECT * FROM r),
  r AS (SELECT 1 AS n)
SELECT * FROM q;
