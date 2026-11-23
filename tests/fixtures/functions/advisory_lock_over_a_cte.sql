-- The kysely lock's shape, answered: a session-level lock projected over a CTE that
-- assigns `lock_timeout` locally — `void` beside the CTE's `1h` (`set_config` answers the
-- value as `SHOW` renders it) — and given back in the same projection, so nothing
-- outlives the test.
-- @test
WITH set_timeout AS (
  SELECT set_config('lock_timeout', '3600000', true) AS config_val
)
SELECT config_val, pg_advisory_lock(3853314791062309107) AS k,
  pg_advisory_unlock(3853314791062309107) AS released
FROM set_timeout;
