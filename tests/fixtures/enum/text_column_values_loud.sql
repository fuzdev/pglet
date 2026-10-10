-- A FROM-clause `VALUES` list folds each column through the same rule, under its own
-- construct word.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for text and mood
SELECT x FROM (VALUES ((SELECT s FROM t LIMIT 1)), ((SELECT m FROM t LIMIT 1))) w(x);
