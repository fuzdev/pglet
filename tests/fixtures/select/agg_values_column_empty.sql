-- Over no rows the aggregate's type is still read: a `COALESCE` default beside `avg` of
-- a `VALUES` list's integer column, and beside `sum` of its `bigint` column, is unified
-- to `numeric`.
-- @test
SELECT COALESCE(avg(x), 7) AS a, COALESCE(sum(y), 7) AS s, COALESCE(sum(x), 7) AS si
FROM (VALUES (1, 2::bigint)) v(x, y) WHERE false;
