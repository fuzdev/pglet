-- The `timestamptz` reader refuses a fraction on top of `23:59:60` the same way.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '2000-06-30 23:59:60.5'::timestamptz;
