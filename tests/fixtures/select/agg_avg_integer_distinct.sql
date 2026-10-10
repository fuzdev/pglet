-- `avg(DISTINCT integer)` is the exact `numeric` quotient over the distinct values, and
-- `sum(DISTINCT bigint)` the exact `numeric` total of them.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (1, 9223372036854775807), (1, 9223372036854775807), (2, 9223372036854775806),
  (4, 9007199254740993), (NULL, NULL);
-- @test
SELECT avg(DISTINCT i) AS a, sum(DISTINCT i) AS s, sum(DISTINCT b) AS sb, avg(DISTINCT b) AS ab FROM t;
