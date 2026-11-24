-- Renaming onto a sequence's name is `42P07`: one relation namespace.
-- @profile-lock
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @expect-error: relation "rp_id_seq" already exists
-- @test
ALTER TABLE rp RENAME TO rp_id_seq;
