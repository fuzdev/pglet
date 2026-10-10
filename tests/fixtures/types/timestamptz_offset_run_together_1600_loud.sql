-- A run-together `hhmm` offset splits into hour 16 and minute 0, and is refused.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00+1600'::timestamptz;
