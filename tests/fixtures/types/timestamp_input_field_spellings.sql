-- The spellings a `timestamp` literal is read through, field by field: the separators
-- between the date and the time (ASCII punctuation such as `/`, `_`, `:` and `#`,
-- whitespace, `T` in either case),
-- a seconds field of 60 at any time of day, an hour of 24, the minute:second form a
-- fraction selects, empty fields, a fraction rounded as a C `double`, the date's other
-- delimiters and digit counts, fields in another order, and an offset that follows the
-- date directly.
-- @test
SELECT
  '2000-01-01 12:34:56'::timestamp         AS space,
  '2000-01-01T12:34:56'::timestamp         AS upper_t,
  '2000-01-01t12:34:56'::timestamp         AS lower_t,
  '2000-01-01/12:34:56'::timestamp         AS slash,
  '2000-01-01_12:34:56'::timestamp         AS underscore,
  '2000-01-01:12:34:56'::timestamp         AS colon,
  '2000-01-01#12:34:56'::timestamp         AS hash,
  '2000-01-01  12:34:56'::timestamp        AS two_spaces,
  '2000-06-30 23:59:60'::timestamp         AS second_60_month_end,
  '2000-06-29 23:59:60'::timestamp         AS second_60_mid_month,
  '2000-01-01 12:30:60.5'::timestamp       AS second_60_with_fraction,
  '2000-01-01 24:00:00'::timestamp         AS hour_24,
  '2000-01-01 12:30.5'::timestamp          AS minute_second_fraction,
  '2000-01-01 12::30'::timestamp           AS empty_minute,
  '2000-01-01 12:'::timestamp              AS empty_minute_at_end,
  '2000-01-01 12:34:56.'::timestamp        AS empty_fraction,
  '2000-01-01 00:00:00.0001255'::timestamp AS double_rounds_down,
  '2000-01-01 00:00:00.0001265'::timestamp AS double_rounds_up,
  '2000-01-01 00:00:00.0000015'::timestamp AS tie_to_even_up,
  '2000-01-01 00:00:00.0000025'::timestamp AS tie_to_even_down,
  '2000/01/01 12:00:00'::timestamp         AS slash_date,
  '2000.01.01 12:00:00'::timestamp         AS dot_date,
  '200-01-01 12:00:00'::timestamp          AS three_digit_year,
  '2000-01-001 12:00:00'::timestamp        AS three_digit_day,
  '2000-01-01-'::timestamp                 AS trailing_delimiter,
  '+05 2000-01-01 12:00:00'::timestamp     AS offset_first,
  '2000-01-01+12:34:56'::timestamp         AS offset_after_date,
  '2000-01-01 12:00:00 z'::timestamp       AS zulu_word
;
