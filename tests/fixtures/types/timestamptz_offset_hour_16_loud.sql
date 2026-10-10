-- A numeric offset of exactly 16 hours is past 18.6's `MAX_TZDISP_HOUR` (15), and the
-- literal is refused rather than stored at a shifted instant.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00+16:00'::timestamptz;
