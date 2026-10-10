-- An alias spelled like a clause keyword (`AS set`) ends no `FROM` list, so the item after
-- the comma is still read: the self-reference is found and the query recurses.
-- @test
WITH RECURSIVE r(n) AS (
  SELECT 1
  UNION ALL
  SELECT n + 1 FROM (SELECT 0) AS set, r WHERE n < 3
)
SELECT n FROM r;
