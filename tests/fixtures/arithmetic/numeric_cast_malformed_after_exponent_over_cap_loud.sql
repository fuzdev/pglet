-- The other half of the pair: the exponent passes `PG_INT32_MAX / 2` at its tenth
-- digit (`9999999999`), so 18.6 never reads the `x` and the malformed spelling answers
-- `22003 value overflows numeric format` — with none of the literal in the message.
-- The in-cap twin is `arithmetic/numeric_cast_malformed_after_exponent_in_cap_loud.sql`.
-- @test
-- @expect-error: exponent past the reader's cap outranks the trailing garbage (22003)
-- @expect-message
SELECT '1e99999999999x'::numeric;
