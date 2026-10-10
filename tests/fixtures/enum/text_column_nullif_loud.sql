-- `nullif` is that equality one constructor in, which is why it takes the **operator**
-- refusal where `COALESCE`/`CASE`/`greatest`/`least` over the same pair take the fold's
-- `42804`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT nullif(m, s) FROM t ORDER BY id;
