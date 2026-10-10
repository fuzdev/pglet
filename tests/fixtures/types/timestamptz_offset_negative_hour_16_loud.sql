-- A west offset is range-checked on its magnitude: `-16:00` is refused as `+16:00` is.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00-16:00'::timestamptz;
