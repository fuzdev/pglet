-- A `numeric` field matches no overload: `integer` takes no implicit cast from it.
-- @expect-error: no make_interval(hours => numeric)
-- @test
SELECT make_interval(hours => 2.5);
