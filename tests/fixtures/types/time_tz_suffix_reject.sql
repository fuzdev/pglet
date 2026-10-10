-- A TIME literal with a time-zone offset: Postgres accepts it (casting to `time`
-- and dropping the zone), but pglet is UTC-only with no time-zone database and
-- rejects the offset (fail loud) — a deliberate pglet-stricter divergence. Since
-- Postgres accepts the query, `@expect-error` passes on any pglet error.
-- @test
-- @expect-error: TIME with a tz offset is rejected (pglet is UTC-only)
SELECT '04:05:06-08:00'::time AS t;
