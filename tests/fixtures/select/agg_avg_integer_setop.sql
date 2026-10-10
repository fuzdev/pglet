-- A set operation unifies an integer `avg` with an integer constant as `numeric`, and
-- `sum(bigint)` with a decimal constant as `numeric`.
-- @setup
CREATE TABLE t (i INT, b BIGINT);
INSERT INTO t VALUES (1, 10), (2, 20);
-- @test
SELECT avg(i) AS a, sum(b) AS s FROM t UNION ALL SELECT 1, 2.5 ORDER BY 1;
