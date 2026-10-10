-- A qualifier a sub-select's own FROM clause carries binds that item, in every position
-- the sub-select evaluates — 18.6's `refnameNamespaceItem` searches the query levels
-- innermost first and stops at the first carrying the name — even where an enclosing
-- level's FROM item has the same name and the same column. The positions a grouping, a
-- window, a sort or a `DISTINCT ON` evaluates over their input's names used to know no
-- relation at all, so the qualifier went to the enclosing frame and read the outer row.
--
-- The other polarity: a qualifier that only an **enclosing** level's FROM item carries is a
-- correlated reference in every one of those positions, and reads the enclosing row —
-- `shd_xy (k, j)` has no `o`. An aggregate's argument beside an inner column, `HAVING`, a
-- grouped select list, a `GROUP BY` key, an `ORDER BY` key whose sign the enclosing row
-- flips, a window key, a windowed select list and a `DISTINCT ON` key — and, last, beside
-- a derived table whose own query names an `o`, which is that query's and not this
-- level's.
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
       (SELECT max(k + o.n) FROM shd_xy) AS arg,
       (SELECT count(*) FROM shd_xy HAVING max(k) < o.n - 9) AS hv,
       (SELECT max(k) + o.n FROM shd_xy) AS grouped_list,
       (SELECT count(*) FROM shd_xy GROUP BY o.n) AS key,
       (SELECT k FROM shd_xy ORDER BY (k - 2) * (o.n - 15) DESC LIMIT 1) AS sort_key,
       (SELECT k FROM shd_xy GROUP BY k ORDER BY (k - 2) * (o.n - 15) DESC LIMIT 1) AS grouped_sort_key,
       (SELECT rank() OVER (ORDER BY (k - 2) * (o.n - 15)) FROM shd_xy WHERE k = 3 ORDER BY 1 LIMIT 1) AS win_key,
       (SELECT k FROM shd_xy ORDER BY (k - 2) * (o.n - 15) DESC, k LIMIT 1) AS sort_below,
       EXISTS (SELECT o.n, rank() OVER (ORDER BY k) FROM shd_xy) AS windowed_list,
       (SELECT DISTINCT ON ((k - 2) * (o.n - 15) > 0) k FROM shd_xy ORDER BY (k - 2) * (o.n - 15) > 0 DESC, k LIMIT 1) AS don_key,
       (SELECT max(s.k + o.n) FROM (SELECT k FROM shd_xy o) s) AS behind_derived,
       (SELECT s.k FROM (SELECT k FROM shd_xy o) s ORDER BY (s.k - 2) * (o.n - 15) DESC LIMIT 1) AS sort_behind_derived
FROM shd_t o ORDER BY y;
