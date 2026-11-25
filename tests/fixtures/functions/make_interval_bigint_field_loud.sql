-- A `bigint` field matches no overload either.
-- @expect-error: no make_interval(hours => bigint)
-- @test
SELECT make_interval(hours => 2::bigint);
