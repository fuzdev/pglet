-- A `HAVING` over a NULL operand's `IN` list holding a scalar subquery is NULL, so the
-- group is dropped.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
-- @test
SELECT x FROM t GROUP BY x HAVING x IN (1, (SELECT 1));
