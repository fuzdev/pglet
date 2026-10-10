-- `transformWithClause` checks duplicate names before it sorts.
-- @expect-error: the duplicate, not the cycle (42712)
-- @expect-message
-- @test
WITH RECURSIVE a AS (SELECT * FROM b),
  b AS (SELECT * FROM a),
  a AS (SELECT 1)
SELECT 1;
