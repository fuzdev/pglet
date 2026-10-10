-- An `UPDATE … SET` whose right-hand side is the refused constructor. This rewrote the
-- stored `sad` to `ok` here against nothing written there — the enum-valued right-hand side
-- is what makes it this gate's rather than the assignment gate's.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, s TEXT NOT NULL, v VARCHAR NOT NULL);
INSERT INTO t VALUES (1, 'sad', 'ok', 'ok'), (2, 'ok', 'ok', 'ok'), (3, 'happy', 'ok', 'ok');

-- @test
-- @expect-error: no common type for mood and text
UPDATE t SET m = greatest(m, s);
