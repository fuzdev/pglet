-- The same bare-name merge reached through a **derived relation** rather than a second
-- base table. A subquery alias and a CTE reference are separate relation-introducing
-- plan nodes from a base scan, so each needs its own qualifier-aware arm; a fix that
-- only descended base scans would leave `d.x` typed as `jdr_a`'s `integer` while the
-- executor sent `jdr_b`'s `text`.
--
-- The derived side is projected **first** so the merge (which takes the first `x` in
-- the flattened join columns — the base table's) mistypes the column under test.
-- @setup
CREATE TABLE jdr_a (x INT, k INT);
CREATE TABLE jdr_b (x TEXT, k INT);
INSERT INTO jdr_a VALUES (1, 7);
INSERT INTO jdr_b VALUES ('z', 7);
-- @test
WITH c AS (SELECT x, k FROM jdr_b)
SELECT d.x AS sub_x, c.x AS cte_x, jdr_a.x AS base_x
FROM jdr_a
JOIN (SELECT x, k FROM jdr_b) d ON d.k = jdr_a.k
JOIN c ON c.k = jdr_a.k;
