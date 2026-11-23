-- An unlock of a key the session holds no session-level lock of answers `false` (18.6
-- adds `WARNING 01000 you don't own a lock of type ExclusiveLock`, which the harness does
-- not compare) — a transaction-level hold of the key included, which no unlock gives
-- back. `pg_advisory_unlock_all()` is `void`, and a NULL argument is NULL.
-- @test
SELECT pg_advisory_xact_lock(42);
SELECT
  pg_advisory_unlock(42) AS xact_held,
  pg_advisory_unlock(42, 42) AS pair,
  pg_advisory_unlock(NULL) AS strict,
  pg_advisory_unlock_all() AS all_released;
