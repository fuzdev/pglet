-- A renamed table's unique index is still enforced under the new name: `23505` on the index's own name.
-- @profile-lock
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @expect-error: duplicate key value violates unique constraint "rp_v_key"
-- @test
ALTER TABLE rp RENAME TO rq;
INSERT INTO rq (v, n) VALUES ('a', 3);
