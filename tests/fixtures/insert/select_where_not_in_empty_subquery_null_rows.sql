-- An `INSERT … SELECT` filtered by `x NOT IN (<empty>)` copies every row, the NULL one
-- included.
-- @setup
CREATE TABLE e (a INT);
CREATE TABLE s (x INT);
INSERT INTO s VALUES (1), (NULL);
CREATE TABLE d (x INT);
-- @test
INSERT INTO d SELECT x FROM s WHERE x NOT IN (SELECT a FROM e);
SELECT count(*) FROM d;
