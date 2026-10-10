-- A range bound that resolves to NULL: `WHERE k > $1` with `$1 = NULL`. The
-- comparison `k > NULL` is NULL for every row, so Postgres returns no rows — NOT
-- an unbounded scan. The IndexRangeScan executor must short-circuit a NULL bound
-- to an empty result, matching Postgres.
-- @profile-lock
-- @setup
CREATE TABLE rng_null (id INT, k INT NOT NULL);
INSERT INTO rng_null VALUES
  (1, 10),
  (2, 20),
  (3, 30);
CREATE INDEX rng_null_k ON rng_null(k);
-- @param $1 int4 NULL
-- @test
SELECT id, k FROM rng_null WHERE k > $1 ORDER BY k, id;
