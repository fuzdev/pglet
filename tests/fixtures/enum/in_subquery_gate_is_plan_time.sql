-- The discriminator between this gate and an eval-time one: **no row reaches it**. The
-- subquery selects nothing and the predicate sits behind a false conjunct, so a gate
-- reading operands off `Value`s has nothing to read and the statement answers
-- `(0 rows)`. 18.6 refuses in parse analysis, whatever the rows.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text, over an empty subquery behind a false conjunct
-- @expect-message
-- @profile-lock
SELECT id FROM w WHERE 1 = 0 AND m IN (SELECT s FROM w WHERE false);
