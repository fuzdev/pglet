-- A `HAVING` over a NULL operand's `NOT IN` list holding an aggregate item is NULL, so the
-- group is dropped.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
-- @test
SELECT count(*) FROM t GROUP BY x HAVING x NOT IN (1, count(*)::int);
