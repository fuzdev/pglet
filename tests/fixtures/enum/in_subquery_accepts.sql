-- The accept polarity, all three shapes in one row set: the same enum on both sides, a
-- **member** literal on the left (which the conversion admits), and a `::text` cast that
-- leaves an identity this engine cannot decide rather than one it can contradict
-- (`EDGES.md` row 104). A gate that refused the door outright would redden here.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
SELECT id,
       (m IN (SELECT m FROM w)) AS same_enum,
       ('ok' IN (SELECT m FROM w)) AS member_left,
       (m::text IN (SELECT s FROM w)) AS cast_left
FROM w ORDER BY id;
