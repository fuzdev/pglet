-- A month outside 1-12 is a field value out of range. 18.6 adds a `DateStyle` hint that
-- pglet does not send.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '2000-13-01 00:00:00'::timestamp;
