-- A session-level lock stacks a hold per call and gives one back per unlock: two locks of
-- key 1 take two unlocks, and a third answers `false` (18.6 adds its `WARNING`, which the
-- harness does not compare). `(bigint)` and `(int4, int4)` are separate keys. Each run
-- of the test gives back every hold it took, so nothing outlives it.
-- @test
SELECT pg_advisory_lock(1);
SELECT pg_advisory_lock(1) AS again, pg_advisory_lock(0, 1) AS pair;
SELECT
  pg_advisory_unlock(1) AS first,
  pg_advisory_unlock(1) AS second,
  pg_advisory_unlock(1) AS third,
  pg_advisory_unlock(1, 0) AS pair_swapped,
  pg_advisory_unlock(0, 1) AS pair,
  pg_advisory_unlock(0, 1::int2) AS pair_again;
