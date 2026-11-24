-- A renamed `SERIAL` column keeps drawing from its sequence, which it still owns under the sequence's old name.
-- @setup
CREATE TABLE cp (id serial PRIMARY KEY, v text UNIQUE, n int CHECK (n > 0), m int, UNIQUE (m, n));
CREATE INDEX cp_lower ON cp (lower(v)) WHERE n > 1;
CREATE TABLE cc (pid int REFERENCES cp (id), pm int, pn int, FOREIGN KEY (pm, pn) REFERENCES cp (m, n));
INSERT INTO cp (v, n, m) VALUES ('a', 5, 1), ('b', 7, 2);
-- @test
ALTER TABLE cp RENAME COLUMN id TO ident;
INSERT INTO cp (v, n, m) VALUES ('c', 9, 3);
SELECT ident, pg_get_serial_sequence('cp', 'ident') AS seq FROM cp ORDER BY ident;
