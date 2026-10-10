-- A `timestamp` literal's year-0 field is the same refusal as a date's, on both engines,
-- ahead of any sub-second rounding.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '0000-12-31 12:00:00.9999999'::timestamp;
