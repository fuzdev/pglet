-- The sub-plan runs before the left operand is evaluated, so the sub-plan's `22P02` is the
-- error, not the operand's division by zero.
-- @setup
CREATE TABLE t1 (x INT);
INSERT INTO t1 VALUES (1);
CREATE TABLE r (a INT);
INSERT INTO r VALUES (1);
-- @expect-error: invalid input syntax for type integer
-- @test
SELECT (1/(x-x)) IN (SELECT (a::text || 'z')::int FROM r) FROM t1;
