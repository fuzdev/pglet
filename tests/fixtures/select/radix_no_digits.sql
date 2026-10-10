-- A radix prefix with no digits (`0x` alone) is a lex error, not the number `0`
-- followed by a column `x` (Postgres's `hexfail` rule). SQLSTATE 42601.
-- @test
-- @expect-error: invalid hexadecimal integer (no digits after prefix)
SELECT 0x;
