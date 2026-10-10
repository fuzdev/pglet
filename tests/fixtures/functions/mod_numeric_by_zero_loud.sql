-- A numeric `mod` by zero: 22012.
-- @test
-- @expect-error: division by zero
SELECT mod(7.5, 0);
