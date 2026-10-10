-- An empty sub-plan answers before the left operand is evaluated, uncorrelated and
-- correlated alike: the operand's division by zero never runs.
-- @setup
CREATE TABLE t1 (x INT);
INSERT INTO t1 VALUES (1);
CREATE TABLE e (a INT);
-- @test
SELECT
  (1/(x-x)) IN (SELECT a FROM e) AS in_e,
  (1/(x-x)) NOT IN (SELECT a FROM e) AS nin_e,
  (1/(x-x)) IN (SELECT a FROM e WHERE a = t1.x) AS in_corr
FROM t1;
