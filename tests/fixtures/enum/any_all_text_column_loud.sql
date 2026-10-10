-- `x op ANY/ALL (array)` is 18.6's `make_scalar_array_op`, which resolves `op` between
-- the left-hand type and the array's **element** type — a comparison position neither
-- operand gate carried an arm for, so the pair was decided at eval time on `Value`
-- kinds (`EDGES.md` rows 114 and 115 both recorded it).
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TYPE m3 AS ENUM ('sad', 'happy', 'ok');
CREATE TABLE w (id INT, m mood NOT NULL, y m3 NOT NULL, s TEXT NOT NULL);
INSERT INTO w VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'happy', 'ok'), (3, 'happy', 'sad', 'zzz');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
-- @profile-lock
SELECT id, (m = ANY(ARRAY[s])) AS a FROM w ORDER BY id;
