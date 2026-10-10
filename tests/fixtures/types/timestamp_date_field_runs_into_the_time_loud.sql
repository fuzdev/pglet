-- With no separator between the day and the hour, the date field takes every digit up to
-- the `:`, so the fields are a date `2000-01-01012` and a time `34:56`. The refusal is the
-- time's hour of 34, raised inside the field loop, before the date's day of `01012` is
-- judged; had the reader split the digits into a day `01` and a time `012:34:56`, the
-- spelling would be a value. The day's own refusal is
-- `timestamp_date_field_runs_into_a_valid_time_loud.sql`.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '2000-01-01012:34:56'::timestamp;
