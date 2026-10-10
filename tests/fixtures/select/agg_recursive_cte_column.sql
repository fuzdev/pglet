-- A recursive query's column is its non-recursive term's type, and it is certain where
-- both terms' are: `avg` over an `integer` one is the exact `numeric`, and `sum` over a
-- `bigint` one `numeric`.
-- @test
WITH RECURSIVE r(x) AS (SELECT 1 UNION ALL SELECT x + 3 FROM r WHERE x < 4),
  rb(y) AS (SELECT 9223372036854775806 UNION ALL SELECT y + 1 FROM rb WHERE y < 9223372036854775807)
SELECT (SELECT avg(x) FROM r) AS a, (SELECT round(avg(x)) FROM r) AS ra,
  (SELECT sum(y) FROM rb) AS s, (SELECT avg(y) FROM rb) AS ab;
