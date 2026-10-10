-- A **correlated** reference orders by the outer frame's enum, not the inner scope's:
-- the frame carries its scope's member orders exactly as it carries its declared types,
-- and without them the outer reference compared lexically while the identical inner
-- reference did not.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL);
CREATE TABLE u (k INT);
INSERT INTO t VALUES (1, 'sad'), (2, 'ok'), (3, 'happy');
INSERT INTO u VALUES (1);

-- @test
SELECT id FROM t o WHERE EXISTS (SELECT 1 FROM u WHERE u.k = 1 AND o.m > 'sad')
ORDER BY id;
