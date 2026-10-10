-- A simple `CASE x WHEN v` label is 18.6's `x = v` — the operator rulebook again, and the
-- `42883` lands even though the branch results are perfectly ordinary integers.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT CASE m WHEN s THEN 1 ELSE 0 END FROM t ORDER BY id;
