-- The string escapes are decoded left to right, so the first failing escape decides the
-- code: \u0000 ahead of a lone surrogate is `22P05`. The reversed order is
-- `unicode_escape_lone_surrogate_before_code_point_zero.sql`.
-- @test
-- @expect-error: the code point zero is first
-- @expect-message
SELECT '["\u0000", "\ud800"]'::jsonb;
