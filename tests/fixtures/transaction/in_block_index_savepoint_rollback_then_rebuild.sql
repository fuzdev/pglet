-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- A unique index built after a savepoint and taken back by ROLLBACK TO leaves the block
-- free to write the committed key; a second index built after that reads the committed
-- row and the block's own: 2 on both engines and both profiles.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_sp;
CREATE TABLE ibi_sp (a INT);
INSERT INTO ibi_sp VALUES (1);
BEGIN;
SAVEPOINT s;
CREATE UNIQUE INDEX ibi_sp_u ON ibi_sp (a);
ROLLBACK TO s;
INSERT INTO ibi_sp VALUES (1);
CREATE INDEX ibi_sp_i ON ibi_sp (a);

-- @test
SELECT count(*) FROM ibi_sp WHERE a = 1;
