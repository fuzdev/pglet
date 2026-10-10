-- Over a sub-plan with a row in it the left operand is evaluated: its division raises.
-- @setup
CREATE TABLE t1 (x INT);
INSERT INTO t1 VALUES (1);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1);
-- @expect-error: division by zero
-- @test
SELECT (1/(x-x)) NOT IN (SELECT a FROM r) FROM t1;
