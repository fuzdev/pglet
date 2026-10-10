-- A scalar subquery on the text side: the comparison gate's reader had no arm for it, so
-- the pair fell through even though the enum side was read.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: no operator for mood > text
-- @expect-message
SELECT m > (SELECT s FROM t WHERE id = 1) FROM t;
