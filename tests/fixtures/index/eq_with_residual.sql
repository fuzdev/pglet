-- Index probe with a residual conjunct: `WHERE k = 10 AND name = 'c'` lifts the
-- `k = 10` equality into an IndexScan and keeps `name = 'c'` in a wrapping Filter.
-- The combined result must match Postgres (the index narrows, the filter refines).
-- @profile-lock
-- @setup
CREATE TABLE idx_resid (id INT, k INT, name TEXT);
INSERT INTO idx_resid VALUES
  (1, 10, 'a'),
  (2, 10, 'c'),
  (3, 10, 'c'),
  (4, 20, 'c');
CREATE INDEX idx_resid_k ON idx_resid(k);
-- @test
SELECT id, k, name FROM idx_resid WHERE k = 10 AND name = 'c' ORDER BY id;
