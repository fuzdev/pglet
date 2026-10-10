-- The range half: `find_index_range` calls the same selector as the equality and
-- ordered-scan paths, so a bound `WHERE j > $1` over a `USING gin` index was
-- lowered to a `Plan::IndexRangeScan` on it and raised the B-tree encoder's `0A000`
-- where Postgres answers rows.
--
-- Not covered by the equality fixture: a fix that gated only `find_index_equality`
-- leaves this one red, and the two lower to different plan nodes.
--
-- The corpus is the ordering fixture's, so the expected answer is checkable by hand
-- against it: ascending jsonb order is `2,1,3`, hence `j > '{"m":3}'` is `1,3`.
-- Not `-- @profile-lock` — see `gin_method_eq_param_jsonb.sql`.
-- @setup
CREATE TABLE gin_rng_j (id int4, j jsonb);
INSERT INTO gin_rng_j (id, j) VALUES
  (1, '{"z":1,"a":2}'),
  (2, '{"m":3}'),
  (3, '{"b":1,"c":2,"d":3}');
CREATE INDEX gin_rng_j_idx ON gin_rng_j USING gin(j);
-- @param $1 jsonb {"m":3}
-- @test
SELECT id, j FROM gin_rng_j WHERE j > $1 ORDER BY id;
