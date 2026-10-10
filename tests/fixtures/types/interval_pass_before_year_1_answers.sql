-- `± interval` runs three passes (months, days, microseconds), and 18.6 checks each
-- pass against its own timestamp range, which starts at Julian day 0, 4714-11-24 BC. A
-- pass that lands before year 1 but inside that range is carried on, so a later pass
-- can bring the result back inside the date domain as a value. The first pass-landing
-- inside the range is Julian day 0 itself, for the months pass (`months_pass_first`)
-- and for the days pass (`days_pass_first`, and its `timestamptz` twin). Rendered
-- through `::text`.
-- @test
SELECT
  (timestamp '0001-01-01' + interval '-12 months 366 days')::text                 AS through_year_zero,
  (timestamp '0001-01-01' + interval '-48000 months 1460970 days')::text          AS through_year_4000_bc,
  (timestamp '0004-02-29' + interval '-48 months 365 days')::text                 AS clamp_year_zero,
  (timestamp '0001-01-01' + interval '-56557 months 1721419 days')::text          AS months_pass_december_4714_bc,
  (timestamp '0001-01-24' + interval '-56558 months 1721449 days')::text          AS months_pass_first,
  (timestamptz '0001-01-24 00:00:00+00' + interval '-56558 months 1721449 days')::text AS tz_months_pass_first,
  (timestamp '0001-01-01' + interval '-1721426 days 41314224 hours')::text        AS days_pass_first,
  (timestamptz '0001-01-01 00:00:00+00' + interval '-1721426 days 41314224 hours')::text AS tz_days_pass_first,
  (timestamp '0001-01-01' + interval '-1 day 86400000000 microseconds')::text     AS day_then_micros
;
