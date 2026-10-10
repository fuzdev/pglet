-- A CTE's set operation is the same door one level in — the construct word stays `UNION`,
-- which is what 18.6 reports.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for text and mood
WITH c AS (SELECT s AS x FROM t UNION ALL SELECT m FROM t) SELECT x FROM c;
