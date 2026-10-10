-- The same coercion at `COALESCE`, where the literal branch is never even reached at run
-- time — which is the point: 18.6 raises it whatever the rows.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok');

-- @test
-- @expect-error: zzz is not a member of mood
-- @expect-message
SELECT COALESCE(m, 'zzz') FROM t;
