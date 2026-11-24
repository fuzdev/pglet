-- `ALTER TABLE … RENAME COLUMN`: the column answers under its new name, its values in place.
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @test
ALTER TABLE cp RENAME COLUMN n TO num;
SELECT id, v, num, m FROM cp ORDER BY id;
