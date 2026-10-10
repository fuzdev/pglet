-- DATE casts: text→date, timestamp→date (time-of-day truncated), date→text.
-- @test
SELECT
  '2024-03-04'::date AS from_text,
  '2024-03-04 15:30:00'::timestamp::date AS from_ts,
  '2024-03-04'::date::text AS to_text;
