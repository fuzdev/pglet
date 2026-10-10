-- A NULL operand of an `IN` list whose item is a window function answers NULL.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (NULL);
-- @test
SELECT x, x IN (1, row_number() OVER ()::int) AS w FROM t;
