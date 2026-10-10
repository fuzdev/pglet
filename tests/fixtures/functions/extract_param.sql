-- EXTRACT over a bound `$1 timestamptz` (the spine's extended-protocol path). The
-- source param carries an explicit `::timestamptz` cast — `extract`/`date_part` have
-- one overload per source type, so an *untyped* param can't resolve the function
-- (Postgres 42725); the cast fixes the source type at parse time and types the bound
-- param. `epoch` is scale 6.
-- @param $1 timestamptz 2024-03-15T13:45:06.789Z
-- @test
SELECT
  extract(year   from $1::timestamptz) AS y,
  extract(month  from $1::timestamptz) AS mo,
  extract(day    from $1::timestamptz) AS d,
  extract(hour   from $1::timestamptz) AS h,
  extract(second from $1::timestamptz) AS sec,
  extract(epoch  from $1::timestamptz) AS ep;
