-- An hour of 24 is accepted only as exactly `24:00:00`; any fraction past it is out of
-- range.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '2000-01-01 24:00:00.5'::timestamp;
