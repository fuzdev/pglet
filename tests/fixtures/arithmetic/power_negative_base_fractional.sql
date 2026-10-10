-- A negative base raised to a non-integer exponent yields a complex result
-- (Postgres 2201F). `(-2) ^ 0.5` resolves to `numeric ^ numeric`; the negative-
-- base/fractional-exponent check fires before the deferred general-power path.
-- @test
-- @expect-error: negative base to a non-integer power
SELECT (-2) ^ 0.5;
