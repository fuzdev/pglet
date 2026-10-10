-- An integer *expression* is an integer argument: `avg(i + 0)`, `avg(-i)`, `avg(i / 2)` and
-- `avg(i * 2)` are `numeric`, `sum(b + 0)` over a `bigint` expression is `numeric`, and
-- `sum(i + i)` is `bigint`.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (3, 10), (4, 20), (6, 9223372036854775807);
-- @test
SELECT avg(i + 0) AS a1, avg(-i) AS a2, avg(i / 2) AS a3, avg(i * 2) AS a4,
  sum(b + 0) AS s1, sum(i + i) AS s2 FROM t;
