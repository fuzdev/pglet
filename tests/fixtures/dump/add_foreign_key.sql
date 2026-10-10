-- ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY must create a real FK: the
-- existing child row (pid=1) validates against the parent, then a later orphan
-- insert (pid=99) is rejected with 23503 — matching Postgres.
-- @profile-lock
-- @setup
CREATE TABLE dparent (id int PRIMARY KEY);
INSERT INTO dparent VALUES (1), (2);
CREATE TABLE dchild (pid int);
INSERT INTO dchild VALUES (1);
ALTER TABLE ONLY dchild
    ADD CONSTRAINT dchild_pid_fkey FOREIGN KEY (pid) REFERENCES dparent(id);
-- @test
INSERT INTO dchild VALUES (99);
-- @expect-error: orphan child violates the FK added by ADD CONSTRAINT
