-- `BETWEEN` desugars to `k >= low AND k <= high` (inclusive both ends) at
-- conversion time, so it plans as an IndexRangeScan with two inclusive bounds for
-- free. The boundary rows (k = 10 and k = 30) must be included, matching Postgres.
-- @profile-lock
-- @setup
CREATE TABLE rng_btw (id INT, k INT NOT NULL, name TEXT);
INSERT INTO rng_btw VALUES
  (1, 9, 'a'),
  (2, 10, 'b'),
  (3, 20, 'c'),
  (4, 30, 'd'),
  (5, 31, 'e');
CREATE INDEX rng_btw_k ON rng_btw(k);
-- @test
SELECT id, k, name FROM rng_btw WHERE k BETWEEN 10 AND 30 ORDER BY k, id;
