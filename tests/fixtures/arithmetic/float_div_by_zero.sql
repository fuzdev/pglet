-- Float division by zero also errors (Postgres 22012), matching the integer
-- path rather than producing infinity.
-- @test
-- @expect-error: division by zero
SELECT 1.0 / 0.0;
