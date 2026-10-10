-- `Plan::DistinctOn` sits **above** the projection, so every column an ON key reads
-- must be readable from the projected row. Here the base column `a` is carried under
-- the name `b` (`a AS b`), so the key `a+0` is re-tied to `b+0` and reads the same
-- values; the sort below the projection orders by base `a`, the dedup groups by the
-- same values, and equal groups stay adjacent.
--
-- Without the re-tie the key bound `a` by bare name to the *output* column `a` (the
-- projected `b`), so the sort and the dedup used different columns, equal groups were
-- no longer adjacent, and the dedup emitted **non-adjacent duplicates** — `5|1 7|1 5|2`
-- against 18.6's `5|1 5|2`, which is not a legal `DISTINCT ON` result of any input.
--
-- The names are deliberately swapped: `b AS a, a AS b` makes every bare spelling
-- ambiguous between the two bindings, so a resolver that picks by name rather than by
-- what the projection *denotes* gets it wrong in one direction or the other.
-- @setup
CREATE TABLE dck (a INT, b INT);
INSERT INTO dck VALUES (1, 5), (1, 7), (2, 5);

-- @test
SELECT DISTINCT ON (a+0) b AS a, a AS b FROM dck ORDER BY a+0, b;
