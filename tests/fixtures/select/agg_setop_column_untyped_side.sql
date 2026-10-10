-- An untyped side of a set operation — a `NULL`, a string constant — takes the other
-- side's type, so the column is that side's `integer` or `bigint` and `avg` over it the
-- exact `numeric`, on whichever side the untyped entry stands.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (1, 9007199254740993), (4, 1);
-- @test
SELECT (SELECT avg(x) FROM (SELECT i AS x FROM t UNION ALL SELECT NULL) q) AS null_right,
  (SELECT sum(x) FROM (SELECT NULL AS x UNION ALL SELECT b FROM t) q) AS null_left,
  (SELECT avg(x) FROM (SELECT i AS x FROM t UNION ALL SELECT '7') q) AS string_right,
  (SELECT sum(x) FROM (SELECT b AS x FROM t UNION ALL SELECT NULL UNION ALL SELECT i FROM t) q)
    AS chained;
