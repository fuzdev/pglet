-- The ISO `T` spelling range-checks its offset like the space spelling.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01T00:00:00+16:00'::timestamptz;
