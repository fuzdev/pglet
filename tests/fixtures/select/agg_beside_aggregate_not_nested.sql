-- The accept side of the nesting refusal: aggregates side by side in one expression are
-- not nested.
-- @setup
CREATE TABLE t (k INT, i INT, b BIGINT);
INSERT INTO t VALUES (1, 1, 1), (1, 2, 2), (2, 5, 5);
CREATE TABLE u (k INT, i INT);
INSERT INTO u VALUES (1, 10), (2, 20);
-- @test
SELECT avg(i) + max(i) AS a, sum(b) - min(b) AS s, count(*) * 2 AS c,
  COALESCE(max(i), 0) + min(i) AS m
FROM t;
