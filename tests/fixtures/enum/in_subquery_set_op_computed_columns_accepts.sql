-- A **set operation** whose sides are computed columns: neither side's descriptor
-- carries an identity (`EDGES.md` row 107), so `unified_enum_type` has nothing to unify
-- and the column's `ColumnDesc::enum_type` is `None` — while each side, read through the
-- reader that walks its projection, is `mood`. Taking the descriptor's `None` for the
-- answer refused this.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
SELECT id, (m IN (SELECT min(m) FROM w UNION SELECT max(m) FROM w)) AS a FROM w ORDER BY id;
