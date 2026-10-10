-- The merged `USING` column's enum decides a **write**, not only a row order: a range
-- predicate over the merged name picks which rows a `DELETE` removes. By label bytes
-- `m > 'sad'` matched nothing at all here (`happy` and `ok` both sort below `sad`), so
-- the `DELETE` removed no row where 18.6 removes two — the surviving rows are the
-- assertion, and they are read back as plain `INT`s.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (m mood NOT NULL, tag TEXT);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES ('sad', 'a'), ('ok', 'b'), ('happy', 'c');

-- @test
DELETE FROM t WHERE id IN (SELECT t2.id FROM t t2 JOIN u USING (m) WHERE m > 'sad');
SELECT id FROM t ORDER BY id;
