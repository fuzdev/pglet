-- @no-txn
-- Drives its own BEGIN/COMMIT, so it opts out of the harness's isolating wrapper.
--
-- A `SET LOCAL` made after a savepoint is taken back by a `ROLLBACK TO` that savepoint
-- (`AtEOSubXact_GUC`), leaving the one made before it; the block's end then drops the
-- `SET LOCAL` altogether. Each value is captured inside the block into a table, so the
-- block can end before the compared read.
-- @setup
DROP TABLE IF EXISTS lt_local;
CREATE TABLE lt_local (at TEXT, v TEXT);
SET lock_timeout = '3s';
BEGIN;
SET LOCAL lock_timeout = '8s';
SAVEPOINT a;
SET LOCAL lock_timeout = '9s';
ROLLBACK TO a;
INSERT INTO lt_local SELECT 'after rollback to', current_setting('lock_timeout');
COMMIT;
INSERT INTO lt_local SELECT 'after commit', current_setting('lock_timeout');
-- @test
SELECT at, v FROM lt_local ORDER BY at;
