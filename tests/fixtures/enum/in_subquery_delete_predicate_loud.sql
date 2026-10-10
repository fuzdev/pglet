-- The door **writes**: a `DELETE` predicate takes the same comparison. What the refused
-- statement leaves is `crates/pglet`'s `enum_identity_gates` — a fixture is scored on
-- the first error its script raises, so it cannot both refuse and then count.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
DELETE FROM w WHERE m IN (SELECT s FROM w);
