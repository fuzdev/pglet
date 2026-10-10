-- `WHERE x NOT IN (<empty>)` keeps every row, the NULL one included.
-- @setup
CREATE TABLE t (x INT);
INSERT INTO t VALUES (1), (NULL);
CREATE TABLE e (a INT);
-- @test
SELECT count(*) FROM t WHERE x NOT IN (SELECT a FROM e);
