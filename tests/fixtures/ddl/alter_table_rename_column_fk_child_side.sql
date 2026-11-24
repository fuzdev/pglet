-- A renamed referencing column: the child's foreign key follows it, and a matching row is admitted.
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @test
ALTER TABLE cc RENAME COLUMN pn TO parent_n;
INSERT INTO cc (pid, pm, parent_n) VALUES (1, 1, 5);
SELECT pg_get_constraintdef(oid) FROM pg_constraint WHERE conname = 'cc_pm_pn_fkey';
