-- `NULL` is an `unknown` at the same position, so a one-element `ARRAY[NULL]` folds to
-- `text[]` exactly as a string literal does — the same rule reached through the other
-- literal shape.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT id FROM w WHERE m = ANY(ARRAY[NULL]);
