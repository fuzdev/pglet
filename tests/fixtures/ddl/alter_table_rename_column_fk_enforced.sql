-- A composite foreign key referencing a renamed column checks against it: `23503`.
-- @profile-lock
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @expect-error: insert or update on table "cc" violates foreign key constraint "cc_pm_pn_fkey"
-- @test
ALTER TABLE cp RENAME COLUMN n TO num;
INSERT INTO cc VALUES (1, 9, 9);
