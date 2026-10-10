-- A dependency cycle of two items is `TopologicalSort`'s `0A000`; it used to fall
-- through to a relation lookup.
-- @expect-error: mutual recursion (0A000)
-- @expect-message
-- @test
WITH RECURSIVE a(n) AS (SELECT 1 UNION ALL SELECT n FROM b),
  b(n) AS (SELECT 1 UNION ALL SELECT n FROM a)
SELECT * FROM a;
