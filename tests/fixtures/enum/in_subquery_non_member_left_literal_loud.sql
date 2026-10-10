-- The **left** side is the one that can still be an `unknown`, and the enum the subquery
-- column names is what 18.6 coerces it to — `enum_in`, which is the whole of the
-- `22P02`. So the conversion runs one way at this position, and it runs.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT id FROM w WHERE 'zzz' IN (SELECT m FROM w);
