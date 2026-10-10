-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- The block stages a row, builds a unique index, and then writes the committed row's key:
-- 23505 on both engines. The index's committed entries and the block's staged rows are
-- two halves of one probe, and this is the committed half behind a staged one.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_st;
CREATE TABLE ibi_st (a INT);
INSERT INTO ibi_st VALUES (1);
BEGIN;
INSERT INTO ibi_st VALUES (2);
CREATE UNIQUE INDEX ibi_st_u ON ibi_st (a);

-- @expect-error: the committed row holds the key
-- @test
INSERT INTO ibi_st VALUES (1);
