-- A qualifier a sub-select's own FROM clause carries binds that item, in every position
-- the sub-select evaluates — 18.6's `refnameNamespaceItem` searches the query levels
-- innermost first and stops at the first carrying the name — even where an enclosing
-- level's FROM item has the same name and the same column. The positions a grouping, a
-- window, a sort or a `DISTINCT ON` evaluates over their input's names used to know no
-- relation at all, so the qualifier went to the enclosing frame and read the outer row.
--
-- This one: `HAVING`, a bare grouped column in it and an aggregate in it. Each read the
-- enclosing row's `n` (`10`, `20`), so no group passed and both answered NULL where 18.6
-- answers `1`.
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
SELECT y,
       (SELECT 1 FROM shd_ev o GROUP BY o.n HAVING o.n = 7) AS h_col,
       (SELECT 1 FROM shd_ev o HAVING max(o.n) = 7) AS h_agg
FROM shd_t o ORDER BY y;
