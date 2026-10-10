-- An `ICONST` is an unsigned integer that **fits `int32`**, so a length above it is the
-- same grammar error as a fractional one rather than the `cannot exceed 10485760` a
-- reader that took an `i64` would have reached for.
-- @no-error-fields: both engines send a `P` and pglet's reader reports one token later —
--   see `types/refused_typmod_char_length_not_an_integer_loud.sql`.
-- @expect-error: a character length above int4
-- @test
SELECT 'ab'::char(2147483648) AS never_reached;
