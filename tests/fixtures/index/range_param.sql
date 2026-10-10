-- Index range read with a BOUND parameter: `WHERE k > $1` over the extended
-- protocol (the spine's real binding path). The planner lifts `k > $1` into an
-- IndexRangeScan whose lower bound is the `$N`; the param type is inferred from
-- the indexed column, and the bound probe must return the same rows as Postgres.
-- @profile-lock
-- @setup
CREATE TABLE rng_param (id INT, k INT NOT NULL, name TEXT);
INSERT INTO rng_param VALUES
  (1, 10, 'a'),
  (2, 20, 'b'),
  (3, 30, 'c'),
  (4, 40, 'd');
CREATE INDEX rng_param_k ON rng_param(k);
-- @param $1 int4 20
-- @test
SELECT id, k, name FROM rng_param WHERE k > $1 ORDER BY k, id;
