-- `ARRAY[…]` resolves through the same fold, and its elements take the same coercion.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT ARRAY[m, 'zzz'] FROM t;
