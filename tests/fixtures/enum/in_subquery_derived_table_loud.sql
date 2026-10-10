-- The subquery column need not be a table column: a FROM-clause `VALUES` inside a
-- derived table types it `text` on both engines, and the reader that answers for it is
-- the wildcard route rather than a target-list entry. Gating only the `SELECT <column>`
-- spelling leaves this one silent.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT id FROM w WHERE m IN (SELECT * FROM (VALUES ('sad')) v);
