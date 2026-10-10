-- EXTRACT over a timestamp: the common fields plus the fractional-scale ones. The
-- return type is numeric with Postgres's exact per-field display scale — `second` and
-- `epoch` scale 6, `milliseconds` scale 3, `microseconds` and the integer fields scale
-- 0, `julian` at the numeric-division default scale. `dow` is 0=Sunday, `isodow`
-- 1=Monday. The default comparison runs both the binary and the text wire format, so a
-- wrong scale fails loud.
-- @test
SELECT
  extract(year         from '2024-03-15 13:45:06.789'::timestamp) AS y,
  extract(month        from '2024-03-15 13:45:06.789'::timestamp) AS mo,
  extract(day          from '2024-03-15 13:45:06.789'::timestamp) AS d,
  extract(hour         from '2024-03-15 13:45:06.789'::timestamp) AS h,
  extract(minute       from '2024-03-15 13:45:06.789'::timestamp) AS mi,
  extract(second       from '2024-03-15 13:45:06.789'::timestamp) AS sec,
  extract(milliseconds from '2024-03-15 13:45:06.789'::timestamp) AS ms,
  extract(microseconds from '2024-03-15 13:45:06.789'::timestamp) AS us,
  extract(quarter      from '2024-03-15 13:45:06.789'::timestamp) AS q,
  extract(week         from '2024-03-15 13:45:06.789'::timestamp) AS w,
  extract(dow          from '2024-03-15 13:45:06.789'::timestamp) AS dow,
  extract(isodow       from '2024-03-15 13:45:06.789'::timestamp) AS isodow,
  extract(doy          from '2024-03-15 13:45:06.789'::timestamp) AS doy,
  extract(decade       from '2024-03-15 13:45:06.789'::timestamp) AS dec,
  extract(century      from '2024-03-15 13:45:06.789'::timestamp) AS cent,
  extract(millennium   from '2024-03-15 13:45:06.789'::timestamp) AS mil,
  extract(isoyear      from '2024-03-15 13:45:06.789'::timestamp) AS isoyear,
  extract(julian       from '2024-03-15 13:45:06.789'::timestamp) AS julian,
  extract(epoch        from '2024-03-15 13:45:06.789'::timestamp) AS ep;
