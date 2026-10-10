-- `AS from` is a column label, so the `b` and `a` after it are columns and not `FROM`
-- items: neither item reads the other, and there is no cycle.
-- @test
WITH RECURSIVE
  a AS (SELECT 1 AS from, b FROM (SELECT 2 AS b) z),
  b AS (SELECT 1 AS from, a FROM (SELECT 3 AS a) y)
SELECT a.b, b.a FROM a, b;
