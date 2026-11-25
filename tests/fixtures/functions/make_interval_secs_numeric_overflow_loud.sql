-- ...and a value too large for one is `22003` too, before the interval's own check.
-- @expect-error: value out of range: overflow
-- @test
SELECT make_interval(secs => 1e400);
