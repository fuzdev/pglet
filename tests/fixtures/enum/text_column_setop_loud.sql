-- A set operation's per-column fold is the same rulebook at a door that is no branch
-- reader at all. This one **writes** through an `INSERT … SELECT` source and through an
-- `IN (…)` predicate, both of which pglet answered where 18.6 refuses.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for text and mood
SELECT s FROM t UNION ALL SELECT m FROM t;
