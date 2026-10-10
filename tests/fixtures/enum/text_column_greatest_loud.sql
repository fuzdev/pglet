-- `greatest`/`least` are the same fold under their own construct word, and the statement
-- is refused whatever the rows hold — the fold is parse analysis, not evaluation.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for mood and text
SELECT greatest(m, s) FROM t ORDER BY id;
