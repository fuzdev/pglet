-- `UNION`, `INTERSECT` and `EXCEPT` type their column as `UNION ALL` does, and `avg` and
-- `sum` over each read a certain `bigint`.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (1, 1), (4, 9007199254740993), (4, 4);
-- @test
SELECT (SELECT avg(x) FROM (SELECT i AS x FROM t UNION SELECT b FROM t) q) AS u,
  (SELECT sum(x) FROM (SELECT b AS x FROM t INTERSECT SELECT i FROM t) q) AS i,
  (SELECT avg(x) FROM (SELECT b AS x FROM t EXCEPT SELECT i FROM t) q) AS e,
  (SELECT sum(x) FROM (SELECT i AS x FROM t UNION ALL SELECT b FROM t ORDER BY 1 LIMIT 5) q) AS l;
