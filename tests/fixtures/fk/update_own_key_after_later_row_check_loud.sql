-- A target row's own foreign key is checked once the statement's rows are written, so a
-- later row's CHECK outranks an earlier row's missing parent.
-- @setup
CREATE TABLE g (id int PRIMARY KEY);
INSERT INTO g VALUES (1);
CREATE TABLE t (id int, gid int REFERENCES g, v int CHECK (v > 0));
INSERT INTO t VALUES (1, 1, 1), (2, 1, 1);
-- @test
-- @expect-error: the second row's CHECK before the first row's key
-- @expect-message
-- @profile-lock
UPDATE t SET gid = CASE WHEN id = 1 THEN 99 ELSE gid END, v = CASE WHEN id = 2 THEN -1 ELSE v END;
