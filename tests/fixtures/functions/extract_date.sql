-- EXTRACT over a date: every field is an integer (scale 0), including `julian` (the
-- Julian Day Number) and `epoch` (whole seconds since 1970-01-01). Time-of-day fields
-- are not defined for a bare date (see extract_wrong_source.sql).
-- @test
SELECT
  extract(year       from '2024-03-15'::date) AS y,
  extract(month      from '2024-03-15'::date) AS mo,
  extract(day        from '2024-03-15'::date) AS d,
  extract(quarter    from '2024-03-15'::date) AS q,
  extract(week       from '2024-03-15'::date) AS w,
  extract(dow        from '2024-03-15'::date) AS dow,
  extract(isodow     from '2024-03-15'::date) AS isodow,
  extract(doy        from '2024-03-15'::date) AS doy,
  extract(decade     from '2024-03-15'::date) AS dec,
  extract(century    from '2024-03-15'::date) AS cent,
  extract(millennium from '2024-03-15'::date) AS mil,
  extract(isoyear    from '2024-03-15'::date) AS isoyear,
  extract(julian     from '2024-03-15'::date) AS julian,
  extract(epoch      from '2024-03-15'::date) AS ep;
