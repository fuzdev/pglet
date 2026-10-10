-- Two range bounds on the same indexed column merge into ONE IndexRangeScan:
-- `WHERE k >= 10 AND k < 30` becomes a single bounded probe (inclusive lower,
-- exclusive upper). The result (the half-open window) must match Postgres.
-- The merge itself is `planner::tests`'; this compares the rows.
-- @profile-lock
-- @setup
CREATE TABLE rng_gelt (id INT, k INT NOT NULL, name TEXT);
INSERT INTO rng_gelt VALUES
  (1, 5, 'a'),
  (2, 10, 'b'),
  (3, 15, 'c'),
  (4, 29, 'd'),
  (5, 30, 'e'),
  (6, 40, 'f');
CREATE INDEX rng_gelt_k ON rng_gelt(k);
-- @test
SELECT id, k, name FROM rng_gelt WHERE k >= 10 AND k < 30 ORDER BY k, id;
