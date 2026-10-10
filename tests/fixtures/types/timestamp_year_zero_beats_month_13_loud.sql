-- The year is judged before the month, so a year 0 beside a month 13 is still a field
-- value out of range, quoting the input.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '0000-13-01 00:00:00'::timestamp;
