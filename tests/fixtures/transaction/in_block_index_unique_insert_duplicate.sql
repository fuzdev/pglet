-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- A unique index built inside a block holds the row committed before the block, so the
-- block's duplicate INSERT is refused where the index was built: 23505 on both engines.
-- pglet used to accept it and publish two rows under the index at COMMIT.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_ins;
CREATE TABLE ibi_ins (a INT);
INSERT INTO ibi_ins VALUES (1);
BEGIN;
CREATE UNIQUE INDEX ibi_ins_u ON ibi_ins (a);

-- @expect-error: the committed row holds the key
-- @test
INSERT INTO ibi_ins VALUES (1);
