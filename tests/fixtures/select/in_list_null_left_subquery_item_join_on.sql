-- A `JOIN … ON` over a NULL operand's `IN` list holding a scalar subquery is NULL, so no
-- pair joins.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1), (2);
-- @test
SELECT count(*) FROM t JOIN r ON t.x IN (1, (SELECT 1));
