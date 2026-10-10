-- An `UPDATE` filtered by `x NOT IN (<empty>)` rewrites every row, the NULL one included.
-- @setup
CREATE TABLE e (a INT);
CREATE TABLE t (x INT, y INT);
INSERT INTO t VALUES (1, 0), (NULL, 0);
-- @test
UPDATE t SET y = 1 WHERE x NOT IN (SELECT a FROM e);
SELECT count(*) FROM t WHERE y = 1;
