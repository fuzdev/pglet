-- A `HAVING` over a NULL operand's `IN (SELECT …)` with rows is NULL, so the group is
-- dropped.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1), (2);
-- @test
SELECT count(*) FROM t GROUP BY x HAVING x IN (SELECT a FROM r);
