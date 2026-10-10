-- The spine's `created_at < $1` shape: a range over a TIMESTAMPTZ indexed column
-- with a bound parameter. The planner lifts `created_at < $1` into an
-- IndexRangeScan with an exclusive upper bound; the bound probe must return the
-- same rows as Postgres.
-- @profile-lock
-- @setup
CREATE TABLE rng_ts (id INT, created_at TIMESTAMPTZ NOT NULL);
INSERT INTO rng_ts VALUES
  (1, '2026-01-01 00:00:00Z'),
  (2, '2026-03-01 00:00:00Z'),
  (3, '2026-06-01 00:00:00Z'),
  (4, '2026-09-01 00:00:00Z');
CREATE INDEX rng_ts_created ON rng_ts(created_at);
-- @param $1 timestamptz 2026-06-01T00:00:00Z
-- @test
SELECT id, created_at FROM rng_ts WHERE created_at < $1 ORDER BY created_at, id;
