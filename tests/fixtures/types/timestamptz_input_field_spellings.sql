-- The `timestamptz` reader over the same field spellings, where the zone matters: an
-- offset written first or straight after the date (`+12:34:56` is an offset, not a time
-- of day), the word `z` before the date and after the time, and a POSIX zone (`z+05` among
-- them) with no daylight-saving part, whose offset counts west (`utc+16` is sixteen hours
-- behind UTC).
-- @test
SELECT
  '2000-01-01 12:34:56'::timestamptz         AS space,
  '2000-01-01t12:34:56'::timestamptz         AS lower_t,
  '2000-01-01/12:34:56+05'::timestamptz      AS slash_with_offset,
  '2000-06-30 23:59:60+05'::timestamptz      AS second_60_with_offset,
  '2000-01-01 24:00:00-05'::timestamptz      AS hour_24_with_offset,
  '2000-01-01 00:00:00.0001255'::timestamptz AS double_rounds_down,
  '2000-01-01+12:34:56'::timestamptz         AS offset_after_date,
  '+05 2000-01-01 00:00:00'::timestamptz     AS offset_first,
  '2000-01-01 +05 00:00:00'::timestamptz     AS offset_between,
  'z 2000-01-01'::timestamptz                AS zulu_first,
  '2000-01-01 00:00:00 z'::timestamptz       AS zulu_after_space,
  '2000-01-01T00:00:00Z+05'::timestamptz     AS posix_zulu_plus_5,
  '2000-01-01 00:00:00 utc+16'::timestamptz  AS posix_utc_plus_16,
  '2000-01-01 x5'::timestamptz               AS posix_one_letter,
  '2000-01-01 00:00:00 abc-167'::timestamptz AS posix_max_hour_east,
  '2000-01-01 00:00:00 abc+05:30'::timestamptz AS posix_minutes,
  '10000-01-01 00:00:00+05'::timestamptz     AS five_digit_year_into_domain
;
