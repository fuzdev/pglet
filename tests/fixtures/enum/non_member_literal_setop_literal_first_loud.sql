-- The same door with the sides swapped: the literal leads and the enum follows, so the
-- coercion is the left entry's. Both entries go through one call, in column order.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT 'zzz' UNION ALL SELECT m FROM t;
