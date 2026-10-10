-- A cast makes an aggregate's argument certain whatever the column under it: over a FROM
-- `unnest` of an `ARRAY[…]` constructor, whose column's type is not certain, `avg(x::int)`
-- is the exact `numeric` and `sum(x::bigint)` `numeric`.
-- @test
SELECT avg(x::int) AS a, sum(x::bigint) AS s FROM unnest(ARRAY[1, 4]) AS v(x);
