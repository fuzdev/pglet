-- A qualifier a sub-select's own FROM clause carries binds that item, even where an enclosing
-- level's FROM item has the same name and the same column (`EDGES.md` row 323).
--
-- This one: the inner item is a reference to a recursive `WITH` query.
-- @setup
CREATE TABLE shd_t (y INT, n INT);
CREATE TABLE shd_u (y INT, n INT);
INSERT INTO shd_t VALUES (1, 10), (2, 20);

-- @test
WITH RECURSIVE o(n) AS (SELECT 5 UNION ALL SELECT o.n + 2 FROM o WHERE o.n < 7)
SELECT y, (SELECT max(o.n) FROM o) AS mx, (SELECT o.n FROM o ORDER BY o.n DESC LIMIT 1) AS top_n
FROM shd_t o ORDER BY y;
