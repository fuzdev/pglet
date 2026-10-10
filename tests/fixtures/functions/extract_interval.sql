-- EXTRACT over an interval, via Postgres's (year, month, day, h/m/s) decomposition:
-- `hour` is the unclamped magnitude, `day` the raw day field, `month` the month
-- remainder (0-11), `year` the whole years. `epoch` counts a year as 365.25 days and a
-- remainder-month as 30 days: `5 days 3 hours` = 442800 s.
-- @test
SELECT
  extract(year         from INTERVAL '2 years 3 months 10 days 4 hours 5 minutes 6.789 seconds') AS y,
  extract(month        from INTERVAL '2 years 3 months 10 days')                                 AS mo,
  extract(day          from INTERVAL '10 days 4 hours')                                          AS d,
  extract(week         from INTERVAL '20 days')                                                  AS w,
  extract(hour         from INTERVAL '50 hours 30 minutes')                                       AS h,
  extract(minute       from INTERVAL '4 hours 5 minutes 6 seconds')                               AS mi,
  extract(second       from INTERVAL '4 hours 5 minutes 6.789 seconds')                           AS sec,
  extract(milliseconds from INTERVAL '6.789 seconds')                                             AS ms,
  extract(quarter      from INTERVAL '8 months')                                                  AS q,
  extract(epoch        from INTERVAL '5 days 3 hours')                                            AS ep,
  -- months-bearing epoch exercises the year/month legs (year = 365.25 d, remainder
  -- month = 30 d) of the formula, not just days/time.
  extract(epoch        from INTERVAL '1 year 2 mons 10 days 3 hours')                             AS ep_months,
  -- a negative interval: the × 4 … ÷ 4 integer trick must carry the sign correctly.
  extract(epoch        from INTERVAL '-1 year -2 mons -10 days')                                  AS ep_neg;
