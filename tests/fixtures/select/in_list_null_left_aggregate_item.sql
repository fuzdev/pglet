-- A NULL operand of an `IN` list whose item is an aggregate answers NULL, as 18.6 does:
-- the item is not evaluated through a walk that cannot evaluate an aggregate.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
-- @test
SELECT NULL::int IN (1, max(x)) AS a FROM t;
