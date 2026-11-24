-- A renamed table's constraints keep their names and keep holding: the primary key's and the unique index's, the `CHECK`, and another table's foreign key now referencing the new name.
-- @setup
CREATE TABLE rp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0));
CREATE INDEX rp_lower ON rp (lower(v)) WHERE n > 1;
CREATE TABLE rc (pid int REFERENCES rp (id), x int);
INSERT INTO rp (v, n) VALUES ('a', 5), ('b', 7);
INSERT INTO rc VALUES (1, 1);
-- @test
ALTER TABLE rp RENAME TO rq;
SELECT c.conname, c.contype::text, t.relname FROM pg_constraint c JOIN pg_class t ON t.oid = c.conrelid WHERE c.conname IN ('rp_pkey', 'rp_v_key', 'rp_n_check', 'rc_pid_fkey') ORDER BY 1;
