-- `analyzeCTE` runs item by item, so the arity error of the item resolved first is the
-- statement's — `c`'s, not the recursive `r`'s raised while `r` parses.
-- @expect-error: the first item's arity error (42P10)
-- @expect-message
-- @test
WITH RECURSIVE c(a, b) AS (SELECT 1),
  r(n, m) AS (SELECT 1 UNION ALL SELECT n + 1 FROM r WHERE n < 3)
SELECT 1;
