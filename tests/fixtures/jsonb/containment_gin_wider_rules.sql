-- The widened containment rules under a **GIN index**, where the operator has two
-- implementations to keep in agreement: the posting-list intersection that picks
-- candidate rows, and the `@>` recheck that decides them.
--
-- `gin_decompose` is deliberately lossy — it collects each object key and scalar
-- leaf with no path qualification, so a candidate is not yet an answer. The
-- soundness property the wider rules must preserve is one-directional: if
-- `container @> contained` then every token of the rhs appears in the lhs, so the
-- intersection never drops a true match. Rows 3 and 4 exercise the two shapes that
-- test it — an array subset (whose numeric elements contribute *no* tokens at all,
-- leaving an empty probe that must match every row and be resolved by the recheck)
-- and the raw-scalar exception.
-- @profile-lock
-- @setup
CREATE TABLE jgin (id INT, d JSONB);
INSERT INTO jgin VALUES
  (1, '{"tags":["x","y"],"n":1}'),
  (2, '{"tags":["y"],"n":2}'),
  (3, '[1,2,3]'),
  (4, '["a","b"]'),
  (5, '{"tags":[{"k":1}]}');
CREATE INDEX jgin_idx ON jgin USING GIN (d);
-- @test
SELECT id FROM jgin WHERE d @> '{"tags":["x"]}'::jsonb OR d @> '[1,3]'::jsonb OR d @> '"a"'::jsonb OR d @> '{"tags":[{"k":1}]}'::jsonb ORDER BY id;
