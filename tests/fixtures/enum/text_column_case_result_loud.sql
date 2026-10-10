-- `CASE`'s **results** are a common-type position (its `WHEN` conditions are not), so the
-- enum/`text` pair is the fold's `42804` here and the operand spelling's `42883` in
-- `text_column_case_operand_loud`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for text and mood
SELECT CASE WHEN id = 0 THEN s ELSE m END FROM t ORDER BY id;
