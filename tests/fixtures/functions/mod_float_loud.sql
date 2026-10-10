-- No float `mod`: 42883.
-- @test
-- @expect-error: function mod(double precision, integer) does not exist
SELECT mod(7.0::float8, 3);
