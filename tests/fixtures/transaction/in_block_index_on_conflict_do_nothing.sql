-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- ON CONFLICT DO NOTHING inside the block, arbitrated by a unique index the block built,
-- skips the committed row's key: one row after the COMMIT on both engines, where pglet
-- used to insert a second.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_ocn;
CREATE TABLE ibi_ocn (a INT, v TEXT);
INSERT INTO ibi_ocn VALUES (1, 'old');
BEGIN;
CREATE UNIQUE INDEX ibi_ocn_u ON ibi_ocn (a);
INSERT INTO ibi_ocn VALUES (1, 'y') ON CONFLICT (a) DO NOTHING;
COMMIT;

-- @test
SELECT a, v FROM ibi_ocn ORDER BY v;
