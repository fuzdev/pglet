-- EXTRACT over a time: hour/minute/second/milliseconds/microseconds and `epoch`
-- (seconds since midnight). `second`/`epoch` are scale 6, `milliseconds` scale 3,
-- `microseconds` scale 0. Date and zone fields are not defined for a time.
-- @test
SELECT
  extract(hour         from '13:45:06.789'::time) AS h,
  extract(minute       from '13:45:06.789'::time) AS mi,
  extract(second       from '13:45:06.789'::time) AS sec,
  extract(milliseconds from '13:45:06.789'::time) AS ms,
  extract(microseconds from '13:45:06.789'::time) AS us,
  extract(epoch        from '13:45:06.789'::time) AS ep;
