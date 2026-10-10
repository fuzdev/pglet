-- `NOT IN (SELECT …)` is the same node negated and the same `=` underneath — 18.6
-- builds the identical `SubLink`, so the negation cannot be a second verdict. Gating the
-- plain spelling alone leaves this one silent, and it is the one that *removes* rows:
-- a non-matching predicate under `NOT` is true for every row.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT id FROM w WHERE m NOT IN (SELECT s FROM w);
