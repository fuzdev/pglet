-- At an operator position the target is the other operand's type, so the same `enum_in`
-- runs — after the operator resolution, never instead of it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT m > 'zzz' FROM t;
