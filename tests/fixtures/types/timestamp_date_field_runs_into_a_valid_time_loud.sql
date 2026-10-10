-- The date field takes every digit up to the `:`, so the fields are a date `2000-01-01012`
-- and a time `14:56`, which is in range. The refusal is the date's day of `01012`, judged
-- after the field loop.
-- @expect-error: date/time field value out of range
-- @expect-message
-- @test
SELECT '2000-01-01012:14:56'::timestamp;
