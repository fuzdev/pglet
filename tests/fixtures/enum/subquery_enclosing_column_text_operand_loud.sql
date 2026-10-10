-- The refuse polarity of `enum/subquery_enclosing_column_projection_accepts`: the
-- enclosing chain answers with the enum, and the `text` operand beside it then has no
-- operator — so the arm's answer is load-bearing in both directions.
-- @expect-error: a text column compared with an enclosing enum through a sub-plan
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok'), (2, 'ok', 'ok'), (3, 'happy', 'zzz');

-- @test
SELECT s IN (SELECT m FROM (VALUES (1)) q) FROM t;
