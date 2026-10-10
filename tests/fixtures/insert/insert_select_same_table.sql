-- INSERT … SELECT from the same table: the source query is fully read before any
-- row is inserted, so the just-inserted rows do NOT feed back into the scan (no
-- Halloween problem). This holds on the auto-commit write path even though the
-- implicit-transaction wrap routes the scan through the pending buffer — the
-- source is materialized before the first staged insert.
-- @profile-lock
-- @setup
CREATE TABLE t (id INT PRIMARY KEY, u INT);
INSERT INTO t VALUES (1, 10), (2, 20);
INSERT INTO t SELECT id + 100, u + 100 FROM t;

-- @test
SELECT * FROM t ORDER BY id;
