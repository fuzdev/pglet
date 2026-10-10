-- And in a partial index predicate, the other persisted-expression position the door
-- reaches.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text in an index predicate over a mixed array
-- @expect-message
CREATE INDEX wix ON w (id) WHERE m = ANY(ARRAY[s, 'ok']);
