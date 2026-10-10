-- A `timestamptz` cast APPLIES an explicit offset, converting the instant to UTC
-- (Postgres `timestamptz_in`); a naive input assumes UTC. The tz-aware
-- counterpart of `timestamp_input_offset_dropped` — this path is load-bearing
-- (the fuz_app matrix depends on it) and must stay unchanged. The harness pins
-- both sessions to `TimeZone=UTC`, so the applied offset yields the same UTC
-- instant on both: `+05` → `07:00`, `-08:30` → `20:30`, fractional seconds kept.
-- @test
SELECT
  '2024-06-15 12:00:00+05'::timestamptz        AS plus5,
  '2024-06-15 12:00:00-08:30'::timestamptz     AS minus830,
  '2024-06-15 12:00:00.123456+05'::timestamptz AS frac,
  '2024-06-15 12:00:00'::timestamptz           AS naive
;
