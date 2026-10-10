-- A **bare** correlated reference into a join outer, which the frame's exact match on
-- synthesized `"relation.column"` names could not find: `w` lives on one side only, so
-- 18.6's `scanRTEForColumn` counts one claimant and binds it, while matching the string
-- `w` against `jbc_a.w` / `jbc_b.y` matched nothing and the statement was `42703`. An
-- over-rejection on ordinary SQL — no quoted name anywhere.
--
-- Its ambiguity polarity is `select/correlated_outer_ref_join_bare_ambiguous.sql` (both
-- sides carrying the name, `42702` on both engines), and the `USING`-merge polarity is
-- `select/correlated_outer_ref_using_merged_column.sql`.
-- @setup
CREATE TABLE jbc_a (id INT, w TEXT);
CREATE TABLE jbc_b (id INT, y TEXT);
CREATE TABLE jbc_z (k TEXT);
INSERT INTO jbc_a VALUES (1, 'hit'), (2, 'miss');
INSERT INTO jbc_b VALUES (1, 'yy'), (2, 'zz');
INSERT INTO jbc_z VALUES ('hit');

-- @test
SELECT jbc_a.id FROM jbc_a JOIN jbc_b ON jbc_b.id = jbc_a.id
WHERE EXISTS (SELECT 1 FROM jbc_z WHERE jbc_z.k = w);
