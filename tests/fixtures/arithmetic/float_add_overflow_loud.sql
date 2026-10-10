-- A finite-operand `float8 + float8` that overflows to infinity fails loud
-- (Postgres `float8_pl` raises "value out of range: overflow", 22003) rather than
-- returning Infinity.
-- @test
-- @expect-error: float add overflow
SELECT '1e308'::float8 + '1e308'::float8;
