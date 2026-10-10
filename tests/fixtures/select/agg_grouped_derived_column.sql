-- A grouped query's column read through a derived table, a nesting and a scalar subquery
-- is the base column it passes through, so its integer type is certain and
-- `avg`/`sum(bigint)` over it are 18.6's exact `numeric` on both legs. The extended leg's
-- descriptor used to drop the grouped column's base table where the simple leg kept it, and
-- the two legs declared different types over one value.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT);
INSERT INTO t VALUES (1, 1, 1), (2, NULL, NULL), (3, 5, 9007199254740993), (4, 2, 2), (5, 2, 2);
-- @test
SELECT (SELECT sum(g) FROM (SELECT b AS g FROM t GROUP BY b) s) AS sum_derived,
  (SELECT avg(g) FROM (SELECT i AS g FROM t GROUP BY i HAVING count(*) > 0) s) AS avg_having,
  (SELECT avg(g) FROM (SELECT i AS g FROM t GROUP BY 1) s) AS avg_ordinal,
  (SELECT avg(g) FROM (SELECT g FROM (SELECT i AS g FROM t GROUP BY i) s1) s) AS avg_nested,
  (SELECT avg(g) FROM (SELECT i AS g, max(k) FROM t GROUP BY i) s) AS avg_beside_aggregate;
