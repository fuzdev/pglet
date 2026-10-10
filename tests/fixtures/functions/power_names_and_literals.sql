-- The unaliased columns are `power` and `pow`; two string literals resolve to float8,
-- a string literal beside an integer to float8 and beside a numeric to numeric.
-- @test
SELECT power(2, 10), pow(2.0, 3), power('2', '3') AS uu, power(2, '3') AS iu,
  power(2.5, '2') AS nu, power(NULL, NULL) AS nn;
