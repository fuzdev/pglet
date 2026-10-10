-- The mask half of `opt_interval`, over a literal that names its own units so the
-- unit rule does not apply. A `DAY` mask zeroes the sub-day remainder, a `YEAR` mask
-- rounds the month count down to a whole year, and a `MINUTE` mask never reaches the
-- higher-order day — the asymmetry that makes this a mask rather than a truncation.
-- @test
SELECT
  INTERVAL '1 day 3 hours' DAY AS d,
  INTERVAL '1 year 3 months' YEAR AS y,
  INTERVAL '1 day' MINUTE AS m,
  INTERVAL '1 year 2 mons 3 days 04:05:06' AS unqualified;
