-- A qualifier a sub-select's own FROM clause carries binds that item, in every position
-- the sub-select evaluates — 18.6's `refnameNamespaceItem` searches the query levels
-- innermost first and stops at the first carrying the name — even where an enclosing
-- level's FROM item has the same name and the same column. The positions a grouping, a
-- window, a sort or a `DISTINCT ON` evaluates over their input's names used to know no
-- relation at all, so the qualifier went to the enclosing frame and read the outer row.
--
-- Control: the enclosing item is named `shd_t`, so no enclosing frame owns `o` and the
-- inner item answered on both engines before the level's names reached these positions.
-- @setup
CREATE TABLE shd_t (y INT, n INT);
CREATE TABLE shd_ev (n INT);
CREATE TABLE shd_w (n INT, g INT);
CREATE TABLE shd_xy (k INT, j INT);
INSERT INTO shd_t VALUES (1, 10), (2, 20);
INSERT INTO shd_ev VALUES (5), (7);
INSERT INTO shd_w VALUES (5, 1), (7, 1), (6, 2), (9, 2);
INSERT INTO shd_xy VALUES (1, 0), (2, 0), (3, 0);

-- @test
SELECT y, (SELECT max(o.n) FROM shd_ev o) AS mx,
       (SELECT o.n FROM shd_ev o ORDER BY o.n DESC LIMIT 1) AS top_n,
       (SELECT count(*) FROM shd_ev o GROUP BY o.n ORDER BY 1 LIMIT 1) AS c
FROM shd_t ORDER BY y;
