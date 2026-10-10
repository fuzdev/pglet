-- The date domain's edges are values on both engines, at each producer that can land
-- on one: the text readers (an explicit offset moving a `timestamptz` onto either edge,
-- and a sub-second rounding landing on the lower one), `date ± integer`,
-- `date ± interval`, `timestamp ± interval`, `date + time` and `date_trunc`. Each is
-- rendered through `::text`, so the comparison is 18.6's own output text.
-- @test
SELECT
  '0001-01-01'::date::text                                                AS d_lo,
  '9999-12-31'::date::text                                                AS d_hi,
  ('0001-01-02'::date - 1)::text                                          AS d_minus,
  (1 + '9999-12-30'::date)::text                                          AS d_plus,
  '0001-01-01 00:00:00'::timestamp::text                                  AS ts_lo,
  '9999-12-31 23:59:59.999999'::timestamp::text                           AS ts_hi,
  '0001-01-01 05:00:00+05'::timestamptz::text                             AS tz_lo,
  '0001-01-01 04:59:59.9999995+05'::timestamptz::text                     AS tz_lo_rounded,
  '9999-12-31 18:59:59.999999-05'::timestamptz::text                      AS tz_hi,
  (date '0001-01-02' - interval '1 day')::text                            AS di_lo,
  (timestamp '9999-12-31 23:59:59.999998' + interval '1 microsecond')::text AS ti_hi,
  (timestamptz '0001-01-01 00:00:01+00' - interval '1 second')::text      AS tzi_lo,
  (timestamp '0001-01-15 00:00:00' + interval '-1 month 30 days')::text   AS ti_through_year_zero,
  (date '9999-12-31' + time '23:59:59.999999')::text                      AS dt_hi,
  date_trunc('week', timestamp '0001-01-03 12:00:00')::text               AS trunc_week,
  date_trunc('decade', timestamp '0010-06-01')::text                      AS trunc_decade
;
