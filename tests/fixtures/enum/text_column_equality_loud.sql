-- Equality is in the family and the member order does not decide it: `m = s` answered the
-- `ok` row here under the label bytes, and 18.6 has no `mood = text` operator either.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT id FROM t WHERE m = s ORDER BY id;
