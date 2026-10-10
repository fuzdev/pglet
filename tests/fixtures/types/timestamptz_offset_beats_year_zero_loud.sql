-- The offset's verdict comes before the year-0 field's: 18.6 decodes the offset inside
-- its field loop and validates the date after it.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '0000-12-31 23:00:00+16'::timestamptz;
