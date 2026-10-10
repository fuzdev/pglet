-- `USING (m)` is `left.m = right.m`, so a `mood` merged against a `text` is the operator
-- refusal and not a merged column. pglet built the column and handed it the enum's order,
-- answering a different **row** than either reading of the old behaviour.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no operator for mood = text
-- @expect-message
SELECT a.id FROM t a JOIN (SELECT id, s AS m FROM t) b USING (m) ORDER BY a.id;
