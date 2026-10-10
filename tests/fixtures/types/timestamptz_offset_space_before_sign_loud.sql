-- Whitespace before the offset's sign does not hide it from the range check.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00 +16:00'::timestamptz;
