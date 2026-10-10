-- A qualifier a sub-select's own FROM clause carries binds that item, in every position
-- the sub-select evaluates — 18.6's `refnameNamespaceItem` searches the query levels
-- innermost first and stops at the first carrying the name — even where an enclosing
-- level's FROM item has the same name and the same column. The positions a grouping, a
-- window, a sort or a `DISTINCT ON` evaluates over their input's names used to know no
-- relation at all, so the qualifier went to the enclosing frame and read the outer row.
--
-- This one writes: a `DELETE` filtered by the sub-select. With the target's own `n` read,
-- only the row whose `n` is 7 went (`DELETE 1`) where 18.6 removes both.
-- @setup
CREATE TABLE shd_t (y INT, n INT);
CREATE TABLE shd_ev (n INT);
CREATE TABLE shd_w (n INT, g INT);
CREATE TABLE shd_xy (k INT, j INT);
INSERT INTO shd_t VALUES (1, 10), (2, 20);
INSERT INTO shd_ev VALUES (5), (7);
INSERT INTO shd_w VALUES (5, 1), (7, 1), (6, 2), (9, 2);
INSERT INTO shd_xy VALUES (1, 0), (2, 0), (3, 0);
CREATE TABLE shd_d (y INT, n INT);
INSERT INTO shd_d VALUES (1, 7), (2, 20);

-- @test
DELETE FROM shd_d WHERE EXISTS (SELECT 1 FROM shd_ev shd_d HAVING max(shd_d.n) = 7);
SELECT y, n FROM shd_d ORDER BY y;
