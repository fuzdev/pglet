-- A derived table may project two columns under one name, and a reference to that name
-- is then ambiguous *within the single frame that owns it* — `42702` on 18.6, in the
-- qualified spelling as in the bare one. pglet took the first match, so this answered a
-- row and the value it compared against was decided by select-list order.
--
-- Counting is **exact**, which is the half that keeps ordinary joins working: a
-- `USING`-merged `id` is one exact claimant sitting ahead of the raw `a.id` / `b.id`, so
-- `select/correlated_outer_ref_using_merged_column.sql` still answers. Count suffix
-- matches too and that fixture goes red with a spurious `42702`.
-- @setup
CREATE TABLE don_d1 (a INT, b INT);
CREATE TABLE don_d2 (k INT);
INSERT INTO don_d1 VALUES (1, 2);
INSERT INTO don_d2 VALUES (1);

-- @expect-error: column reference "x" is ambiguous (42702)
-- @test
SELECT 1 FROM (SELECT a AS x, b AS x FROM don_d1) d WHERE EXISTS (SELECT 1 FROM don_d2 WHERE don_d2.k = d.x);
