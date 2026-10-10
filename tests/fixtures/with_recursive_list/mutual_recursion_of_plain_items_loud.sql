-- A cycle of three items none of which reads itself.
-- @expect-error: a three-item cycle (0A000)
-- @expect-message
-- @test
WITH RECURSIVE a AS (SELECT * FROM b),
  b AS (SELECT * FROM c),
  c AS (SELECT * FROM a)
SELECT 1;
