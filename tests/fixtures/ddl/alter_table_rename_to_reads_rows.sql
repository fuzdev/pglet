-- `ALTER TABLE … RENAME TO`: the rows, and the serial default's sequence, follow the table to its new name.
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @test
ALTER TABLE rp RENAME TO rq;
INSERT INTO rq (v, n) VALUES ('c', 9);
SELECT id, v, n FROM rq ORDER BY id;
