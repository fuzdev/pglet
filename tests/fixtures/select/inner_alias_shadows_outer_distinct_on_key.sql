-- A qualifier a sub-select's own FROM clause carries binds that item, in every position
-- the sub-select evaluates — 18.6's `refnameNamespaceItem` searches the query levels
-- innermost first and stops at the first carrying the name — even where an enclosing
-- level's FROM item has the same name and the same column. The positions a grouping, a
-- window, a sort or a `DISTINCT ON` evaluates over their input's names used to know no
-- relation at all, so the qualifier went to the enclosing frame and read the outer row.
--
-- This one: a `DISTINCT ON` key that no select-list entry equals. Under a column list the
-- planner computes it as a junk column and the dedup reads that by name, so the key's
-- qualifier reaches the sort beneath; under `*` the dedup evaluates the key itself, over
-- the sort's output names. Over the enclosing row's constant parity every inner row fell
-- in one group (`3` groups counted where 18.6 counts `2`, the sort having split them).
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
       (SELECT count(*) FROM (SELECT DISTINCT ON (o.n % 2) o.g FROM shd_w o ORDER BY o.n % 2, o.g) s) AS groups,
       (SELECT DISTINCT ON (o.n % 2) o.g FROM shd_w o ORDER BY o.n % 2 DESC, o.g DESC LIMIT 1) AS first_g,
       (SELECT count(*) FROM (SELECT DISTINCT ON (o.n % 2) * FROM shd_w o) s) AS star_groups
FROM shd_t o ORDER BY y;
