-- A non-hex digit in a `\x…` bytea literal is `22023` (invalid_parameter_value):
-- `invalid hexadecimal digit: "Z"`, matching Postgres's `byteain` hex path — not
-- the `22P02` the other scalar text parsers use.
-- @test
-- @expect-error: invalid hexadecimal digit is 22023, not 22P02
SELECT '\xZZ'::bytea;
