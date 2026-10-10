-- `± interval` runs three passes (months, days, microseconds), and 18.6 checks each
-- pass against its own timestamp range, which ends at 294276-12-31. A pass that lands
-- between year 10000 and that end is carried on, so a later pass can bring the result
-- back inside the date domain as a value: the `9999-12-31` "no end date" sentinel plus a
-- span that nets inside the domain. A months pass landing on 294276-12-31 (`months_pass_last`)
-- or 294276-12-01 (`months_pass_first_of_month_last`) and a days pass landing on
-- 294276-12-31 (`days_pass_last`) are inside the range; the day-of-month clamps read a
-- leap February in 10000 (divisible by 400) and a common one in 10100 (not). Rendered
-- through `::text`.
-- @test
SELECT
  (date '9999-12-31' + interval '1 day -1 microsecond')::text                    AS date_day_micro,
  (date '9999-12-31' - interval '-1 day 1 microsecond')::text                    AS date_minus,
  (date '9999-12-31' + interval '1 month -31 days')::text                        AS date_month_day,
  (timestamp '9999-12-31 12:00:00' + interval '1 day -13 hours')::text           AS ts_day_hours,
  (timestamptz '9999-12-31 00:00:00+00' + interval '1 day -1 second')::text      AS tz_day_second,
  (timestamptz '9999-12-31 00:00:00+00' - interval '-1 month 31 days')::text     AS tz_minus_month_day,
  (timestamp '9999-12-31' + interval '1 month -31 days')::text                   AS ts_month_day,
  (timestamp '9999-12-01' + interval '1 month -1 day')::text                     AS ts_first_of_month,
  (timestamp '9999-12-15' + interval '1 month -30 days')::text                   AS ts_mid_month,
  (timestamp '9999-12-31' + interval '12 months -366 days')::text                AS ts_year_days,
  (timestamp '9999-12-31' + interval '1000 years -365243 days')::text            AS ts_millennium_days,
  (timestamp '9999-12-31' + interval '3411324 months -103830043 days')::text     AS months_pass_last,
  (timestamp '9999-01-01' + interval '3411335 months -103830377 days')::text     AS months_pass_first_of_month_last,
  (timestamp '9999-12-31' + interval '103830043 days -2491921032 hours')::text   AS days_pass_last,
  (timestamp '9999-01-31' + interval '13 months -60 days')::text                 AS clamp_leap_10000,
  (timestamp '9996-02-29' + interval '48 months -365 days')::text                AS leap_day_10000,
  (timestamp '9996-02-29' + interval '1248 months -36889 days')::text            AS common_feb_10100,
  (timestamptz '9999-12-31 23:00:00+00' + interval '1 hour -1 microsecond')::text AS tz_micro_only
;
