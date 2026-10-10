-- A `timestamptz` literal's year field is judged as written, before its offset moves
-- the instant: this one's UTC reading is in year 1, and both engines still refuse the
-- year-0 field.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '0000-12-31 23:00:00-05'::timestamptz;
