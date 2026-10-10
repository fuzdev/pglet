-- A parameter array under a cast is a cast array: `avg` over its `unnest` is the exact
-- `numeric`.
-- @param $1 int4[] {1,4}
-- @test
SELECT avg(x) AS a, sum(y) AS s FROM unnest($1::int[], '{5000000000}'::bigint[]) AS u(x, y);
