-- Strict: a NULL argument answers NULL, not the `void` a taken lock answers.
-- @test
SELECT
  pg_advisory_xact_lock_shared(NULL) AS one_null,
  pg_advisory_xact_lock_shared(NULL, 1) AS pair_null,
  pg_advisory_xact_lock_shared(1) AS taken;
