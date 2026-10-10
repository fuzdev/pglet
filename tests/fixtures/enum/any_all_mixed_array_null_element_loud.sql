-- The same rule with a `NULL` in place of the label, and the shape that says the
-- predicate is about the **element** and not about the source text: a bare `NULL` beside
-- a typed element reaches the gate already cast to the fold's target by this engine's own
-- `unify_common_type_branches`, so a reader that looked only for a literal saw a cast
-- and deferred.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text over an array holding NULL beside a text column
-- @expect-message
-- @profile-lock
SELECT id, (m = ANY(ARRAY[s, NULL])) AS a FROM w ORDER BY id;
