-- The same door with a **second enum** on the subquery side rather than a `text` one —
-- `EDGES.md` row 115's family, which shares row 114's arm list. Two enums have no
-- operator between them either, so the door refuses `42883` where a label-byte
-- comparison would answer.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = m3
-- @expect-message
SELECT id FROM w WHERE m IN (SELECT y FROM w);
