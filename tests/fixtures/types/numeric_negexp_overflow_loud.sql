-- A negative-exponent literal whose display scale exceeds the numeric format's
-- 14-bit dscale field fails loud with `22003` ("value overflows numeric format"),
-- matching Postgres. This is the direct `::numeric` door to the jsonb path's case: a
-- parse that clamps the scale instead keeps a phantom that renders `0` yet compares
-- non-zero. The parse refuses a display scale past that field itself (`parse_decimal`'s
-- `NUMERIC_DSCALE_MAX` cap), before any weight check runs.
-- @expect-error: numeric negative-exponent scale overflow (22003)
-- @expect-message
-- @test
SELECT '1e-200000'::numeric;
