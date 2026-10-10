-- A second numeric offset is read as its own field, and its range is judged before the
-- reader notices it is a second offset.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '2000-01-01 00:00:00+05+16'::timestamptz;
