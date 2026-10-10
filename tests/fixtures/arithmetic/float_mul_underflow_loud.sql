-- A nonzero-operand `float8 * float8` that underflows to zero fails loud
-- (Postgres `float8_mul` raises "value out of range: underflow", 22003) rather than
-- silently returning 0.
-- @test
-- @expect-error: float multiply underflow
SELECT '1e-200'::float8 * '1e-200'::float8;
