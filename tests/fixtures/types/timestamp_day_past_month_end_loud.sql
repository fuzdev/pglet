-- A day past the end of its month is a field value out of range.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '2000-02-30 12:00:00'::timestamptz;
