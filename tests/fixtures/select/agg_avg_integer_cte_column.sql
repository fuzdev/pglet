-- A CTE's integer column is an integer argument: `avg` is `numeric`, and `sum` over its
-- `bigint` column is `numeric`.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (3, 9223372036854775807), (4, 9223372036854775807), (6, 1);
-- @test
WITH c (x, y) AS (SELECT i, b FROM t)
SELECT avg(x) AS a, sum(y) AS s, avg(y) AS ay FROM c;
