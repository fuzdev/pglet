-- The `ANY`-over-the-array-side comparison **writes**: an `UPDATE` or `DELETE` predicate
-- takes the same one. Member position removes the row `'ok'` outranks and the label bytes
-- remove none, so the surviving row set says which comparator ran.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'happy'), (2, 'sad'), (3, 'ok');
DELETE FROM t WHERE 'ok' > ANY(ARRAY[m]);

-- @test
SELECT id, m::text AS label FROM t ORDER BY id;
