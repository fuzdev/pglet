-- The same pair at a **common-type** fold rather than an operator, where the refusal is
-- the conversion's `42846` and not `42883`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: COALESCE cannot convert text[] to mood[]
-- @expect-message
SELECT id, COALESCE(ARRAY[m], ARRAY['ok']) AS a FROM w ORDER BY id;
