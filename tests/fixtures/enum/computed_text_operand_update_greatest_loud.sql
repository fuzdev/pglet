-- The branch reader's half of the same reach, at the other write: the fold resolved a
-- target over the enum column alone and rewrote a stored member to it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: GREATEST has no common type for mood and a computed text side
UPDATE t SET m = greatest(m, lower(s));
