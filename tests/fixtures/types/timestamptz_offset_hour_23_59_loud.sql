-- An offset past 16 hours by more than its hour field (`+23:59`) is refused too.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00+23:59'::timestamptz;
