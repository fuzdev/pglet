-- The equality half of the same hole: `single_column_index_for` has three consumers,
-- and the ordered-scan pushdown is only one of them. A bound `WHERE j = $1` over a
-- `USING gin` index was lowered to a `Plan::IndexScan` on that index, which then
-- encoded a B-tree probe key and raised `0A000` (*"data type jsonb has no default
-- operator class for a B-tree index"*) where Postgres answers rows — from DDL both
-- engines accept, with the index built and populated.
--
-- The bound `$1` is the whole reason this shape is reachable: `is_row_independent`
-- lifts a `Literal` / `Parameter` / `Bound` into a probe, and every *written*
-- spelling of a jsonb value is a `Cast`, which it declines. So `WHERE j =
-- '{"m":3}'::jsonb` never probed and was always right; only the extended protocol
-- reaches this.
--
-- The column is deliberately **nullable**: the equality path carries no NOT-NULL
-- gate (unlike the ORDER BY pushdown), so this also pins that the fix is not the
-- pushdown's gate in disguise. The NULL row must be excluded (`NULL = x` is NULL),
-- and two matching rows straddle a non-matching one so a one-row reply cannot pass.
-- Not `-- @profile-lock` — this plans to `Filter(Scan)` on both profiles, lowered to
-- no index on either; a build that lowers it to the GIN index reddens this run *and*
-- the lock run's completeness check 1.
-- @setup
CREATE TABLE gin_eq_j (id int4, j jsonb);
INSERT INTO gin_eq_j (id, j) VALUES
  (1, '{"m":3}'),
  (2, '{"a":1}'),
  (3, '{"m":3}'),
  (4, NULL);
CREATE INDEX gin_eq_j_idx ON gin_eq_j USING gin(j);
-- @param $1 jsonb {"m":3}
-- @test
SELECT id, j FROM gin_eq_j WHERE j = $1 ORDER BY id;
