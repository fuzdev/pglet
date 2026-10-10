-- Deriving the identity is not enough on its own: a derivation that found *an* enum and
-- dropped **which** enum accepts this, where 18.6 has no operator for the pair
-- (`EDGES.md` row 115).
-- @expect-error: two different enums through a derived table's computed column
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok'), (2, 'ok', 'happy'), (3, 'happy', 'sad');

-- @test
SELECT w.id FROM w, (SELECT min(y) AS z FROM w w2) v WHERE w.m = v.z;
