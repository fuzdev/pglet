-- A renamed table's `CHECK` still holds: `23514`, under its own name.
-- @profile-lock
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @expect-error: new row for relation "rq" violates check constraint "rp_n_check"
-- @test
ALTER TABLE rp RENAME TO rq;
INSERT INTO rq (v, n) VALUES ('z', 0);
