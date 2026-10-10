-- A seconds field of 60 is accepted while the time of day stays at or below
-- `24:00:00`; a fraction on top of `23:59:60` carries it past, which 18.6's
-- `time_overflows` refuses as a field value out of range.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '2000-06-30 23:59:60.5'::timestamp;
