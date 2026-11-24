-- The foreign key referencing a renamed table checks against it under its new name: `23503`.
-- @profile-lock
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @expect-error: insert or update on table "rc" violates foreign key constraint "rc_pid_fkey"
-- @test
ALTER TABLE rp RENAME TO rq;
INSERT INTO rc VALUES (99, 1);
