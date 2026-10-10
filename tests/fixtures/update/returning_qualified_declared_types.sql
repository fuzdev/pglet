-- `RETURNING` projects through the same descriptor walk, against the target
-- table's columns, so a qualified `t.c` there was declared `text` as well —
-- reaching every driver that reads a write's returned row by declared type.
-- @profile-lock
-- @setup
CREATE TABLE rqd (id INT PRIMARY KEY, n INT, ok BOOL);
INSERT INTO rqd VALUES (1, 10, false);

-- @test
UPDATE rqd SET n = 11, ok = true WHERE rqd.id = 1 RETURNING rqd.id, rqd.n, rqd.ok;
