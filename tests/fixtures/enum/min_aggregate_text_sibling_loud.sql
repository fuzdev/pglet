-- `min`/`max` return one of their argument's own values unchanged, so the result carries
-- the argument's enum — the arm that keeps `COALESCE(min(m), max(s))` from reading as two
-- plain `text` branches.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for mood and text
SELECT COALESCE(min(m), max(s)) FROM t;
