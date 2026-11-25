-- Seconds whose microseconds pass `double precision` are `float8_mul`'s `22003`.
-- @expect-error: value out of range: overflow
-- @test
SELECT make_interval(secs => 1e303);
