-- A rename taken back to a savepoint, and one kept: the table answers under the kept name.
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @test
ALTER TABLE rp RENAME TO rq;
SAVEPOINT s;
ALTER TABLE rq RENAME TO rx;
ROLLBACK TO SAVEPOINT s;
SELECT count(*) FROM rq;
