-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- An UPDATE inside the block moving a row onto a key another committed row holds, under
-- a unique index the block built: 23505 on both engines.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_upd;
CREATE TABLE ibi_upd (a INT);
INSERT INTO ibi_upd VALUES (1), (2);
BEGIN;
CREATE UNIQUE INDEX ibi_upd_u ON ibi_upd (a);

-- @expect-error: the committed row holds the key
-- @test
UPDATE ibi_upd SET a = 1 WHERE a = 2;
