-- A parameter under a cast is a typed cell of a `VALUES` column, and one whose type the
-- client declared is too: the column is `integer` and `avg` over it the exact `numeric`.
-- @param $1 int4 4
-- @test
SELECT avg(x) AS a, round(avg(x)) AS r FROM (VALUES (1), ($1::int)) v(x);
