-- A trailing `.` ends the time of day with an empty fraction, and the `+16` after it is
-- read as an offset and refused.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00.+16'::timestamptz;
