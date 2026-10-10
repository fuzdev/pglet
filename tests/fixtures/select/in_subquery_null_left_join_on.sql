-- A `JOIN … ON` over a NULL operand's `IN (SELECT …)` with rows is NULL, so no pair joins.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1), (2);
-- @test
SELECT t.x FROM t JOIN r ON t.x IN (SELECT a FROM r);
