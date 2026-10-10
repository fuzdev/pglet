-- The refuse polarity of the same widening, and the reason it is not "defer on every
-- computed column": a projected expression whose identity the walk decides to be **no
-- enum** keeps refusing. `lower(s)` is `text` on both engines.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text over a computed sub-plan column
-- @expect-message
SELECT id FROM w WHERE m IN (SELECT lower(s) FROM w);
