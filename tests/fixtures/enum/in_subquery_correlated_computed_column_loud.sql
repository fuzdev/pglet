-- The refuse polarity of the leaf above: an enclosing level's **text** column is decided
-- too, so reaching the outer namespace does not turn the door into a deferral.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text over a correlated computed sub-plan column
-- @expect-message
SELECT id FROM w w1 WHERE w1.m IN (SELECT COALESCE(w1.s, 'ok') FROM w w2);
