-- The offset's verdict comes before a month out of range, which 18.6 validates after
-- its field loop.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-13-01 00:00:00+16'::timestamptz;
