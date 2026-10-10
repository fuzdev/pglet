-- A qualifier a sub-select's own FROM clause carries binds that item, in every position
-- the sub-select evaluates — 18.6's `refnameNamespaceItem` searches the query levels
-- innermost first and stops at the first carrying the name — even where an enclosing
-- level's FROM item has the same name and the same column. The positions a grouping, a
-- window, a sort or a `DISTINCT ON` evaluates over their input's names used to know no
-- relation at all, so the qualifier went to the enclosing frame and read the outer row.
--
-- This one: the inner item named `o` is each kind of FROM item a level can name — a derived
-- table, a CTE reference, `VALUES`, `unnest`, `generate_series`, either side of a join, a
-- `USING` join, a table under its own name, and a quoted alias — each in an aggregate's
-- argument, where pglet read the enclosing row's `10`, `20`.
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
WITH o AS (SELECT n FROM shd_ev)
SELECT y,
       (SELECT max(o.n) FROM (SELECT n FROM shd_ev) o) AS derived,
       (SELECT max(o.n) FROM o) AS cte,
       (SELECT max(o.n) FROM (VALUES (5), (7)) o(n)) AS vals,
       (SELECT max(o.n) FROM unnest(ARRAY[5, 7]) o(n)) AS unnest_item,
       (SELECT max(o.n) FROM generate_series(5, 7) o(n)) AS series,
       (SELECT max(o.n) FROM shd_xy JOIN shd_ev o ON true) AS join_right,
       (SELECT max(o.n) FROM shd_ev o JOIN shd_xy ON true) AS join_left,
       (SELECT max(o.n) FROM shd_ev o JOIN shd_ev p USING (n)) AS join_using,
       (SELECT max(o.n) FROM shd_xy LEFT JOIN shd_ev o ON true) AS join_outer,
       (SELECT max(shd_t.n) FROM shd_ev shd_t) AS own_alias,
       (SELECT max("O".n) FROM shd_ev "O") AS quoted
FROM shd_t o ORDER BY y;
