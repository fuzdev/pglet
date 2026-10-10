-- Range on an indexed column plus a residual filter on another column:
-- `WHERE k > 20 AND name = 'x'` lifts the `k > 20` range into an IndexRangeScan
-- and keeps `name = 'x'` in a wrapping Filter. The combined result must match
-- Postgres (the index narrows, the filter refines).
-- @profile-lock
-- @setup
CREATE TABLE rng_resid (id INT, k INT NOT NULL, name TEXT);
INSERT INTO rng_resid VALUES
  (1, 10, 'x'),
  (2, 25, 'x'),
  (3, 25, 'y'),
  (4, 30, 'x'),
  (5, 30, 'y');
CREATE INDEX rng_resid_k ON rng_resid(k);
-- @test
SELECT id, k, name FROM rng_resid WHERE k > 20 AND name = 'x' ORDER BY id;
