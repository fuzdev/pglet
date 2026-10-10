-- A grouped NULL operand of `IN (SELECT …)` over a sub-select with rows answers NULL.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1), (2);
-- @test
SELECT x IN (SELECT a FROM r) AS a FROM t GROUP BY x;
