-- Zero to a negative power: 2201F.
-- @test
-- @expect-error: zero raised to a negative power is undefined
SELECT power(0, -1);
