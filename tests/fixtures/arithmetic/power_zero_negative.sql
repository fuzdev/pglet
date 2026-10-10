-- `0 ^ negative` is undefined (Postgres 2201F, not a division-by-zero code). The
-- `(-1)` is parenthesized so the exponent lexes as a number, not a `^-` operator.
-- @test
-- @expect-error: zero raised to a negative power
SELECT 0 ^ (-1);
