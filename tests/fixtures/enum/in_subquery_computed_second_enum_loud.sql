-- The same walk over a **second enum**: `min(y)` carries `m3`'s identity, so the pair
-- names both enums and 18.6's message does too. Reading the descriptor's `None` as
-- `text` reported `mood = text` here, which is the wrong pair.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = m3 over an aggregate sub-plan column
-- @expect-message
SELECT id FROM w WHERE m IN (SELECT min(y) FROM w);
