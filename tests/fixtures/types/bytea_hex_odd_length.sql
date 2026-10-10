-- An odd number of hex digits in a `\x…` bytea literal is `22023`
-- (invalid_parameter_value): `invalid hexadecimal data: odd number of digits`,
-- matching Postgres — not the `22P02` the other scalar text parsers use.
-- @test
-- @expect-error: odd-length hex data is 22023, not 22P02
SELECT '\xDEA'::bytea;
