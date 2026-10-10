-- 18.6's `ValidateDate` judges the year before the month, so a year-0 date with a
-- month past 12 is the year's refusal, without the `DateStyle` hint a bad month alone
-- carries. Both are 22008 with the same primary message on 18.6, where only the hint
-- tells the order apart; pglet sends no hint, and its messages differ instead.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '0000-13-01'::date;
