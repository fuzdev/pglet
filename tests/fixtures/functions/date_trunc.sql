-- date_trunc over timestamp / date / interval sources. Zeroes every finer field;
-- month/day set to 1; `week` → Monday of the ISO week; `century`/`millennium` use
-- Postgres's "no year 0" rounding (century 21 = 2001). A `date` source implicit-casts
-- to `timestamp` (the result is a timestamp, not a date). The interval variant zeroes
-- the same cascade over the (months, days, micros) decomposition.
-- @test
SELECT
  date_trunc('hour',       '2024-03-15 12:34:56.789'::timestamp) AS h,
  date_trunc('day',        '2024-03-15 12:34:56.789'::timestamp) AS d,
  date_trunc('month',      '2024-03-15 12:34:56'::timestamp)     AS mo,
  date_trunc('quarter',    '2024-08-15 12:34:56'::timestamp)     AS q,
  date_trunc('year',       '2024-03-15 12:34:56'::timestamp)     AS y,
  date_trunc('week',       '2024-03-15 12:34:56'::timestamp)     AS w,
  date_trunc('decade',     '2024-03-15 00:00:00'::timestamp)     AS dec,
  date_trunc('century',    '2024-03-15'::date)                   AS cent,
  date_trunc('millennium', '2024-03-15'::date)                   AS mil,
  date_trunc('day',        '2024-03-15'::date)                   AS date_src,
  date_trunc('hour',       INTERVAL '5 days 3 hours 27 minutes') AS iv_hour,
  date_trunc('day',        INTERVAL '5 days 3 hours 27 minutes') AS iv_day,
  date_trunc('month',      INTERVAL '2 years 3 months 10 days')  AS iv_month;
