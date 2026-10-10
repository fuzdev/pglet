-- `integer = boolean` has no operator (`42883`). Postgres has no int↔bool cast at
-- all, so this never silently means "nonzero".
-- @setup
CREATE TABLE t (n INT);
INSERT INTO t VALUES (1);
-- @test
-- @expect-error: no integer = boolean operator
SELECT * FROM t WHERE n = true;
