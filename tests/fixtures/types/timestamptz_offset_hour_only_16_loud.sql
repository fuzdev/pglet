-- An hour-only offset (`+16`) is range-checked like the `hh:mm` spelling.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00+16'::timestamptz;
