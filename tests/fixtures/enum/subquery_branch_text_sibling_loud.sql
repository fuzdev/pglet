-- The gate's *other* branch reader: a bare scalar subquery, whose verdict carried the
-- column's `TypeOid` — `text` for an enum — and never its identity. The enum branch does
-- not have to be a column.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for mood and text
SELECT COALESCE((SELECT m FROM t LIMIT 1), s) FROM t ORDER BY id;
