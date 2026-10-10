-- The specific `22003` overflow propagates from a number nested inside an object,
-- not just a top-level scalar. `1e-200000` is the phantom case: its display scale
-- (200000) far exceeds the numeric format's 14-bit dscale field, so Postgres rejects
-- it. A parse that clamps the scale instead truncates the significant digit out and
-- keeps an internally-inconsistent value that renders `0` yet compares non-zero.
-- @expect-error: nested jsonb number overflows numeric format (22003)
-- @expect-message
-- @test
SELECT '{"n": 1e-200000}'::jsonb;
