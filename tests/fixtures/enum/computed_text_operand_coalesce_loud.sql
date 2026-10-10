-- The common-type fold over a computed text branch — the shape the `UPDATE` write rides
-- on, read rather than written.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'OK'), (2, 'ok', 'OK'), (3, 'happy', 'OK');

-- @test
-- @expect-error: COALESCE has no common type for mood and a computed text branch
SELECT COALESCE(m, lower(s)) FROM t;
