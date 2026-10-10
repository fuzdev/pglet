-- An array literal's element reads its offset through the element type's reader.
-- @expect-error: time zone displacement out of range
-- @expect-message
-- @test
SELECT '{"2000-01-01 00:00:00+16:00"}'::timestamptz[];
