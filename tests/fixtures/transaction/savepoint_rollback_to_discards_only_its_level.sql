-- @no-txn
-- Drives its own BEGIN/COMMIT, so it opts out of the harness's isolating
-- transaction wrapper (and keeps its own DROP-first cleanup).
--
-- The core of the feature: `ROLLBACK TO` discards what the savepoint's level
-- wrote and **nothing else** — the row before it survives, the block stays open,
-- and a row written after the rollback commits with it.
-- @setup
DROP TABLE IF EXISTS sp_level CASCADE;
CREATE TABLE sp_level (id INT);
BEGIN;
INSERT INTO sp_level VALUES (1);
SAVEPOINT s;
INSERT INTO sp_level VALUES (2);
ROLLBACK TO s;
INSERT INTO sp_level VALUES (3);
COMMIT;

-- @test
SELECT id FROM sp_level ORDER BY id;
