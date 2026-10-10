-- A qualifier a sub-select's own FROM clause carries binds that item, even where an enclosing
-- level's FROM item has the same name and the same column (`EDGES.md` row 323).
--
-- This one: the inner item is a catalog view, and the enclosing `o` has a `relname` too. The
-- enclosing row's `zz` is what a level that forgot the view's name answers.
-- @setup
CREATE TABLE cvq_w (k INT, n INT);
CREATE TABLE cvq_t3 (y INT, relname TEXT, n INT, k INT);
INSERT INTO cvq_w VALUES (1, 100), (2, 200);
INSERT INTO cvq_t3 VALUES (1, 'zz', 10, 50), (2, 'zz', 20, 60);

-- @test
SELECT y, (SELECT max(o.relname::text) FROM pg_class o WHERE o.relname = 'cvq_w') AS rel
FROM cvq_t3 o ORDER BY y;
