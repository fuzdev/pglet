-- A plain `timestamp` cast DROPS an explicit offset in the input, keeping the
-- literal wall-clock fields (Postgres `timestamp_in`) — it must NOT shift the
-- instant by the offset. This is the input-side mirror of the offset-aware text
-- *output* asymmetry (a plain `timestamp` renders without a trailing offset): a
-- plain-`timestamp` target carries no zone, so any offset in the input is
-- discarded rather than applied. Covers a whole-hour `+05`, a fractional-hour
-- `-08:30`, a `+00:30`, fractional seconds, and a naive (offset-less) input —
-- every case renders the wall-clock `12:00:00` exactly as written.
-- @test
SELECT
  '2024-06-15 12:00:00+05'::timestamp        AS plus5,
  '2024-06-15 12:00:00-08:30'::timestamp     AS minus830,
  '2024-06-15 12:00:00+00:30'::timestamp     AS plus0030,
  '2024-06-15 12:00:00.123456+05'::timestamp AS frac,
  '2024-06-15 12:00:00'::timestamp           AS naive
;
