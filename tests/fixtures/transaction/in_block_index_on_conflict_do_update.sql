-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- ON CONFLICT DO UPDATE inside the block, arbitrated by a unique index the block built,
-- updates the committed row: one row, (1, new), on both engines.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_ocu;
CREATE TABLE ibi_ocu (a INT, v TEXT);
INSERT INTO ibi_ocu VALUES (1, 'old');
BEGIN;
CREATE UNIQUE INDEX ibi_ocu_u ON ibi_ocu (a);
INSERT INTO ibi_ocu VALUES (1, 'new') ON CONFLICT (a) DO UPDATE SET v = excluded.v;
COMMIT;

-- @test
SELECT a, v FROM ibi_ocu ORDER BY v;
