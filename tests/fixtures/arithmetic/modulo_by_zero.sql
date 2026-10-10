-- `x % 0` is a division-by-zero (Postgres 22012), not a silent 0.
-- @test
-- @expect-error: modulo by zero
SELECT 7 % 0;
