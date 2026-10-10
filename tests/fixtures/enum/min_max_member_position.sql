-- `min`/`max` are the two aggregates whose result type is their argument's, so they are
-- the two that carry the argument's enum: the minimum is the **first declared** member,
-- not the lexically smallest label.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');

-- @test
SELECT min(m)::text AS lo, max(m)::text AS hi FROM t;
