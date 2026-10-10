-- The plain side does not have to be `text`. A `varchar` column is category `S` like
-- `text`, so the pair is the same refusal under 18.6's own spelling of the type name.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood = character varying
-- @expect-message
SELECT id FROM t WHERE m = v ORDER BY id;
