-- A negative base to a non-integer power: 2201F.
-- @test
-- @expect-error: a negative number raised to a non-integer power yields a complex result
SELECT pow(-8, 0.5::float8);
