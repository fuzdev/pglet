-- `date + integer` past 18.6's own range (5874897-12-31) is its `date out of range`,
-- 22008, the upper-side twin of `date_minus_int_past_range_loud`.
-- @expect-error: date out of range
-- @expect-message
-- @test
SELECT '9999-12-31'::date + 2147483647;
