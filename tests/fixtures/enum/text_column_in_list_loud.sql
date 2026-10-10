-- An `IN (list)` element is 18.6's `x = elem`, so it reaches the **operator** rulebook
-- rather than the common-type one and takes `42883`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT id FROM t WHERE m IN (s) ORDER BY id;
