-- A partial index's predicate is persisted the same way, and its membership then decided
-- which rows a `UNIQUE` index refused — a write, under an index 18.6 never built.
-- `@profile-lock` rather than a waiver: the refusal is the DDL-time stored-expression
-- walk's, which index selection never reaches, but the identity rule moved operand and type
-- gates and that is exactly the class one build profile can reach and the other cannot —
-- so the stripped server is made to answer rather than argued with.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @profile-lock
-- @expect-error: no common type for mood and text
CREATE UNIQUE INDEX tix ON t (id) WHERE greatest(m, s) > 'ok';
