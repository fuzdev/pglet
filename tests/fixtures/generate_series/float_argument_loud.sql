-- No `double precision` signature, and no implicit cast to `numeric`.
-- @expect-error: function generate_series(double precision, integer) does not exist
-- @test
SELECT * FROM generate_series(1.0::float8, 3);
