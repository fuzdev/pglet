-- A unique key over a renamed column still holds: `23505`.
-- @profile-lock
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @expect-error: duplicate key value violates unique constraint "cp_v_key"
-- @test
ALTER TABLE cp RENAME COLUMN v TO word;
INSERT INTO cp (word, n, m) VALUES ('a', 3, 3);
