-- The typed element that decides the fold can be a **second enum**, and then 18.6 names
-- both: the `unknown` adopts `m3`, not `mood`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = m3 over a mixed array
-- @expect-message
-- @profile-lock
SELECT id, (m = ANY(ARRAY[y, 'ok'])) AS a FROM w ORDER BY id;
