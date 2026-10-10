-- A `VALUES` list's integer column under `sum`/`avg`. Every cell of the column is one the
-- coercion pass types as written, so the column's type is certain: `avg` is the exact
-- `numeric` and `sum` of a `bigint` column the exact `numeric` total, past the `bigint`
-- range, with a cast or without one. `sum` of an `integer` column is `bigint`.
-- @test
SELECT sum(x) AS s, avg(x) AS a, avg(x::int) AS ac, sum(y) AS sy, sum(y::bigint) AS syc,
  avg(y) AS ay
FROM (VALUES (1, 9223372036854775807::bigint), (2, 9223372036854775807), (4, 5)) v(x, y);
