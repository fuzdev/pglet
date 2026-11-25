-- A `numeric` `secs` converts through the cast to `double precision`, which a value too
-- small for one is `22003` at.
-- @expect-error: value out of range: underflow
-- @test
SELECT make_interval(secs => 1e-400);
