-- The time of day is read before the offset that follows it, so an hour of 25 is refused
-- ahead of an out-of-range `+16`.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '2000-01-01 25:00:00+16'::timestamptz;
