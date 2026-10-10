-- Strict: a NULL argument answers NULL — not the `void` a taken lock answers, which is
-- the empty text (`advisory_xact_lock_void.sql`), so both legs tell the two apart.
-- @test
SELECT
  pg_advisory_xact_lock(NULL) AS one_null,
  pg_advisory_xact_lock(NULL, 1) AS pair_null,
  pg_advisory_xact_lock(1) AS taken;
