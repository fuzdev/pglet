-- The same `42846` at the set-operation door, under that door's own construct word.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE v (id INT, m mood NOT NULL, y m3 NOT NULL);
INSERT INTO v VALUES (1, 'ok', 'sad'), (2, 'ok', 'happy'), (3, 'ok', 'ok');

-- @test
-- @expect-error: cannot convert m3 to mood
-- @expect-message
SELECT m FROM v UNION ALL SELECT y FROM v;
