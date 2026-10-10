-- The same `42703` through the **other** frame kind, and it is a different code path: an
-- outer *join* scope has no single relation identity, so it addresses its columns by
-- synthesized `"relation.column"` strings and owns every relation one of those names
-- prefixes. `o` is such a relation and has no `nosuch`, so 18.6 says
-- `42703 column o.nosuch does not exist`; pglet said `42P01 "o"`.
--
-- Its accept polarity is `select/qualifier_valid_join_correlated.sql` (a correlated
-- reference into an outer join alias that *does* resolve), and its reject neighbour is
-- `zzz.k` over the same join — `42P01` on both engines before and after, which is what
-- shows the ownership test reads the frame's names rather than waving qualifiers through.
-- @setup
CREATE TABLE jfl_u (id INT, n TEXT);
CREATE TABLE jfl_o (uid INT, k INT);
CREATE TABLE jfl_x (k INT);
INSERT INTO jfl_u VALUES (1, 'a');
INSERT INTO jfl_o VALUES (1, 5);
INSERT INTO jfl_x VALUES (5);

-- @expect-error: column o.nosuch does not exist (42703)
-- @test
SELECT u.id FROM jfl_u u JOIN jfl_o o ON o.uid = u.id WHERE EXISTS (SELECT 1 FROM jfl_x WHERE jfl_x.k = o.nosuch);
