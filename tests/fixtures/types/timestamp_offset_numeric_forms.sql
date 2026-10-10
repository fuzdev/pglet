-- A plain `timestamp` decodes the same numeric offset spellings and then drops them,
-- keeping the written fields.
-- @test
SELECT
  '2000-01-01 00:00:00+15:59:59'::timestamp     AS east_hms,
  '2000-01-01 00:00:00-15:59:59'::timestamp     AS west_hms,
  '2000-01-01 00:00:00+05:30:15'::timestamp     AS east_hms_small,
  '2000-01-01T00:00:00.123+05:30:15'::timestamp AS t_frac_hms,
  '2000-01-01 00:00:00+1559'::timestamp         AS run_together,
  '2000-01-01 00:00:00+5'::timestamp            AS one_digit_hour,
  '2000-01-01 00:00:00 +05'::timestamp          AS space_before_sign,
  '2000-01-01 00:00:00+15:'::timestamp          AS empty_minute
;
