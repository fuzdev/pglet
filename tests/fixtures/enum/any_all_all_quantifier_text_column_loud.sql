-- `ALL` is the same operator resolution as `ANY` — `make_scalar_array_op` differs only
-- in `useOr` — so gating one spelling and not the other leaves half the door open.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT id, (m = ALL(ARRAY[s])) AS a FROM w ORDER BY id;
