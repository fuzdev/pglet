-- A renamed column in the catalog: `information_schema.columns` lists it under the new name, at its position.
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @test
ALTER TABLE cp RENAME COLUMN n TO num;
SELECT column_name::text, ordinal_position::int FROM information_schema.columns WHERE table_name = 'cp' ORDER BY ordinal_position;
