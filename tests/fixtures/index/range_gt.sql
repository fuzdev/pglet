-- Index-accelerated range read: `WHERE k > 20` over a single-column index lifts
-- into an IndexRangeScan (a bounded index probe) instead of a full scan. The
-- ORDER BY runs normally on top — range acceleration only narrows the WHERE — so
-- the result must match Postgres byte-for-byte. `(k, id)` is a total order for a
-- deterministic positional compare.
-- @profile-lock
-- @setup
CREATE TABLE rng_gt (id INT, k INT NOT NULL, name TEXT);
INSERT INTO rng_gt VALUES
  (1, 5, 'a'),
  (2, 20, 'b'),
  (3, 21, 'c'),
  (4, 50, 'd'),
  (5, 100, 'e');
CREATE INDEX rng_gt_k ON rng_gt(k);
-- @test
SELECT id, k, name FROM rng_gt WHERE k > 20 ORDER BY k, id;
