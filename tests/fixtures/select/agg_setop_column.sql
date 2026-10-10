-- A set operation's column is of the sides' common type, and it is certain where both
-- sides' are: `integer` beside `bigint` is `bigint`, so `avg` is the exact `numeric` and
-- `sum` `numeric`, past the `bigint` range.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (1, 9223372036854775807), (4, 9007199254740993), (NULL, NULL);
-- @test
SELECT (SELECT avg(x) FROM (SELECT i AS x FROM t UNION ALL SELECT i FROM t) q) AS avg_int,
  (SELECT round(avg(x)) FROM (SELECT 1 AS x UNION ALL SELECT 4) q) AS round_avg,
  (SELECT sum(x) FROM (SELECT i AS x FROM t UNION ALL SELECT b FROM t) q) AS sum_mixed,
  (SELECT avg(x) FROM (SELECT b AS x FROM t WHERE i = 4 UNION ALL SELECT i FROM t) q) AS avg_mixed,
  (SELECT sum(x) FROM (SELECT i AS x FROM t UNION ALL SELECT 5000000000) q) AS sum_literal;
