-- The controls: a decimal that reads as a subnormal is in range, and an exact zero with
-- any exponent is zero.
-- @test
SELECT '1e-40'::real AS r, '4e-324'::float8 AS d, '0e-999'::float8 AS z,
       '-0'::float8 AS nz, ' 1.5 '::real AS padded;
