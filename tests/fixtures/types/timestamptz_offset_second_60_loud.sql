-- An offset's second field is range-checked on its own: `+15:59:60`.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00+15:59:60'::timestamptz;
