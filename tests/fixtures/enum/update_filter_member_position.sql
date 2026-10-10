-- The `UPDATE`'s `WHERE` takes the member order too — a *write* decided by the
-- comparison, and a different eval entry (`scan_matching_rows`) than the `SELECT`'s.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE t (id INT, m mood NOT NULL, g INT);
INSERT INTO t VALUES (1, 'sad', 0), (2, 'ok', 0), (3, 'happy', 0);
UPDATE t SET g = 9 WHERE m > 'sad';

-- @test
SELECT id, g FROM t ORDER BY id;
