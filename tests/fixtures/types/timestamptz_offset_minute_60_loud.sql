-- An offset's minute field is range-checked on its own: `+15:60` is under 16 hours in
-- total and still refused.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00+15:60'::timestamptz;
