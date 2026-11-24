-- The indexes over a renamed column follow it: an expression key's column and a partial predicate's.
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @test
ALTER TABLE cp RENAME COLUMN v TO word;
ALTER TABLE cp RENAME COLUMN n TO num;
SELECT indexname, indexdef FROM pg_indexes WHERE tablename = 'cp' ORDER BY 1;
