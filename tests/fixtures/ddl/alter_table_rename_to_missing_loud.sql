-- Renaming a table that does not exist is `42P01`.
-- @profile-lock
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @expect-error: relation "nosuch" does not exist
-- @test
ALTER TABLE nosuch RENAME TO zz;
