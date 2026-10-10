-- The `ARRAY[…]` **pair** spelling of the same element rule — `EDGES.md` row 114's array
-- residual. `ARRAY['ok']` is `text[]`, so the pair is `mood[] < text[]` and has no
-- operator; while an unresolved element erased the whole array's identity, the pair fell
-- back to the OID rule and ordered the two arrays element-wise.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood[] < text[]
-- @expect-message
-- @profile-lock
SELECT id, (ARRAY[m] < ARRAY['ok']) AS a FROM w ORDER BY id;
