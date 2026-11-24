-- Renaming onto an enum's name is `42710`: the table's row type takes the new name too.
-- @profile-lock
-- @setup
CREATE TYPE rmood AS ENUM ('a');
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @expect-error: type "rmood" already exists
-- @test
ALTER TABLE rp RENAME TO rmood;
