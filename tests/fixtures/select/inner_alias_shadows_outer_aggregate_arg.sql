-- A qualifier a sub-select's own FROM clause carries binds that item, in every position
-- the sub-select evaluates — 18.6's `refnameNamespaceItem` searches the query levels
-- innermost first and stops at the first carrying the name — even where an enclosing
-- level's FROM item has the same name and the same column. The positions a grouping, a
-- window, a sort or a `DISTINCT ON` evaluates over their input's names used to know no
-- relation at all, so the qualifier went to the enclosing frame and read the outer row.
--
-- This one: an aggregate's argument. The enclosing `o` is `shd_t (y, n)` holding 10 and 20;
-- the inner `o` is `shd_ev (n)` holding 5 and 7. pglet answered the enclosing row's `n`
-- (`10`, `20` for `max`; `20`, `40` for `sum`) where 18.6 aggregates the inner rows.
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
       (SELECT max(o.n) FROM shd_ev o) AS mx,
       (SELECT min(o.n) FROM shd_ev o) AS mn,
       (SELECT sum(o.n) FROM shd_ev o) AS sm,
       (SELECT avg(o.n) FROM shd_ev o) AS av,
       (SELECT count(DISTINCT o.n) FROM shd_ev o) AS cd,
       (SELECT max(o.n) FROM shd_ev o WHERE o.n < 100) AS mw
FROM shd_t o ORDER BY y;
