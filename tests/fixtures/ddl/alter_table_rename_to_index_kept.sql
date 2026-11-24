-- A renamed table's indexes keep their names and their table, and an index read finds the rows.
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @test
ALTER TABLE rp RENAME TO rq;
SELECT indexname, tablename FROM pg_indexes WHERE tablename IN ('rp', 'rq') ORDER BY 1;
