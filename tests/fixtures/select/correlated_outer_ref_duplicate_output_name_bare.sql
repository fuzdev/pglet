-- The **bare** spelling of `select/correlated_outer_ref_duplicate_output_name_qualified.sql`,
-- and a separate arm: the qualified one tests frame ownership before counting, this one
-- counts at the first frame carrying the name. Both are `42702` on 18.6; both answered a
-- row here, off select-list order.
--
-- The control against over-counting is the level rule: two *different* frames each
-- carrying a bare `v` are shadowing rather than ambiguity, and
-- `select/correlated_outer_ref_shadowed_across_levels.sql` pins that the innermost wins.
-- @setup
CREATE TABLE dob_d1 (a INT, b INT);
CREATE TABLE dob_d2 (k INT);
INSERT INTO dob_d1 VALUES (1, 2);
INSERT INTO dob_d2 VALUES (1);

-- @expect-error: column reference "x" is ambiguous (42702)
-- @test
SELECT 1 FROM (SELECT a AS x, b AS x FROM dob_d1) d WHERE EXISTS (SELECT 1 FROM dob_d2 WHERE dob_d2.k = x);
