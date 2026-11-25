-- Seconds whose microseconds pass `bigint` are `22008`.
-- @expect-error: interval out of range
-- @test
SELECT make_interval(secs => 1e300);
