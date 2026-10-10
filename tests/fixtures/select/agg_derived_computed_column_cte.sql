-- A `WITH` query's computed column is certain as a derived table's is — under a column
-- list, read twice, and through a second `WITH` query computing over the first.
-- @setup
CREATE TABLE t (k INT, b BIGINT);
INSERT INTO t VALUES (1, 1), (2, 4), (3, 9007199254740993);
-- @test
WITH c(x) AS (SELECT b + 0 FROM t), c2 AS (SELECT x * 1 AS y FROM c)
SELECT (SELECT avg(x) FROM c) AS a, (SELECT sum(x) FROM c) AS s, (SELECT avg(y) FROM c2) AS ay,
  (SELECT sum(y) FROM c2) AS sy, (SELECT round(avg(y)) FROM c2) AS r;
