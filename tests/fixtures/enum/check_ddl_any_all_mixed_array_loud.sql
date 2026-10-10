-- The mixed array at DDL time: a `CHECK` over it was **recorded** here and refused there,
-- so the catalog dumped to SQL stock 18.6 will not load.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text in a CHECK over a mixed array
-- @expect-message
CREATE TABLE ck (m2 mood NOT NULL, s2 TEXT NOT NULL, CHECK (m2 = ANY(ARRAY[s2, 'ok'])));
