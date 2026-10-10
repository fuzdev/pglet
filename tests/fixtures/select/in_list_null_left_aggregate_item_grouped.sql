-- A grouped NULL operand of an `IN` list holding an aggregate item answers NULL.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
-- @test
SELECT x IN (1, count(*)::int) AS a FROM t GROUP BY x;
