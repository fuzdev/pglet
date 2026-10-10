-- The numeric offset spellings 18.6's `DecodeTimezone` accepts, each applied to the
-- instant: an `hh:mm:ss` offset east and west, a run-together `hhmm` and `hmm`, a
-- one-digit hour, whitespace around the sign, empty and signed minute fields, and the
-- `Z` and `hh`/`hh:mm` controls.
-- @test
SELECT
  '2000-01-01 00:00:00+15:59:59'::timestamptz     AS east_hms,
  '2000-01-01 00:00:00-15:59:59'::timestamptz     AS west_hms,
  '2000-01-01 00:00:00+05:30:15'::timestamptz     AS east_hms_small,
  '2000-01-01 12:00:00-05:30:15'::timestamptz     AS west_hms_small,
  '2000-01-01T00:00:00.123+05:30:15'::timestamptz AS t_frac_hms,
  '2000-01-01 00:00:00+1559'::timestamptz         AS run_together,
  '2000-01-01 00:00:00+155'::timestamptz          AS run_together_hmm,
  '2000-01-01 00:00:00+0530'::timestamptz         AS run_together_0530,
  '2000-01-01 00:00:00+00000005'::timestamptz     AS run_together_long,
  '2000-01-01 00:00:00+5'::timestamptz            AS one_digit_hour,
  '2000-01-01 00:00:00 +05'::timestamptz          AS space_before_sign,
  '2000-01-01 00:00:00+  05'::timestamptz         AS space_after_sign,
  '2000-01-01 00:00:00+05:3'::timestamptz         AS one_digit_minute,
  '2000-01-01 00:00:00+15:'::timestamptz          AS empty_minute,
  '2000-01-01 00:00:00+15:-0'::timestamptz        AS signed_zero_minute,
  '2000-01-01 00:00:00-0'::timestamptz            AS negative_zero,
  '2000-01-01 00:00:00+15'::timestamptz           AS hh,
  '2000-01-01 00:00:00+15:30'::timestamptz        AS hh_mm,
  '2000-01-01 00:00:00-15:59'::timestamptz        AS west_hh_mm,
  '2000-01-01 00:00:00Z'::timestamptz             AS zulu
;
