-- A finite-operand `float8 ^ float8` whose result overflows to infinity is a
-- value-out-of-range error (Postgres 22003).
-- @test
-- @expect-error: float power overflow
SELECT 1e300::float8 ^ 2::float8;
