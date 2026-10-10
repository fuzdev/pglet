-- A cycle beside an item that depends on nothing: the free item sorts first and the
-- cycle is still found.
-- @expect-error: a cycle after a free item (0A000)
-- @expect-message
-- @test
WITH RECURSIVE a AS (SELECT 1 AS n),
  b AS (SELECT * FROM c),
  c AS (SELECT * FROM b)
SELECT * FROM a;
