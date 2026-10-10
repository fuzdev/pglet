-- `timestamp + INTERVAL '1 month'` is real calendar arithmetic (interval is a
-- 3-field span): the month pass shifts the month, clamps the day-of-month to the
-- target month's length, and carries across the year boundary. `2020-01-31 + 1 month`
-- clamps to `2020-02-29` (leap-aware); `2020-12-15 + 1 month` rolls to `2021-01-15`.
-- Differential vs Postgres.
-- @test
SELECT '2020-01-01 12:00:00'::timestamp + INTERVAL '1 month'  AS plus_month,
       '2020-01-31 12:00:00'::timestamp + INTERVAL '1 month'  AS leap_clamp,
       '2020-12-15 00:00:00'::timestamp + INTERVAL '1 month'  AS cross_year,
       '2020-03-31 12:00:00'::timestamp - INTERVAL '1 month'  AS minus_month;
