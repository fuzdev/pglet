-- `0::numeric ^ negative` is a power-domain error (Postgres 2201F, "zero raised to
-- a negative power is undefined"), NOT a division-by-zero — numeric_power raises it
-- before reaching the zero-base division path.
-- @test
-- @expect-error: numeric zero to a negative power
SELECT 0::numeric ^ (-1);
