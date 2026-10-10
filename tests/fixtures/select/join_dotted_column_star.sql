-- A join's namespace is `(relation, column)` **pairs**, not synthesized
-- `relation.column` strings, and this is the shape that says why: `jdc_s`'s column is
-- quoted into existence as `"jdc_w.v"`, byte-identical to the name a join would
-- synthesize for `jdc_w`'s own `v`. Re-parsing that name filed the source column under
-- the pair `("jdc_w", "v")`, so `SELECT *` answered `jdc_w`'s `v` twice — silent, both
-- engines accepting.
--
-- Its single-relation control is `select/join_dotted_column_single_relation.sql`: a
-- scan's names are the user's own, so nothing collides there and both engines agree
-- with or without the pair keying. That is what puts the defect in the join's
-- addressing rather than in the column name.
-- @setup
CREATE TABLE jdc_w (id INT, v TEXT);
CREATE TABLE jdc_s (id INT, "jdc_w.v" TEXT);
INSERT INTO jdc_w VALUES (1, 'a'), (2, 'b');
INSERT INTO jdc_s VALUES (1, 'srcval'), (2, 'srcval2');

-- @test
SELECT * FROM jdc_w JOIN jdc_s s ON s.id = jdc_w.id ORDER BY jdc_w.id;
