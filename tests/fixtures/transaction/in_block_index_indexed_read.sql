-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- An equality read inside the block through an index the block built answers the
-- committed row on both engines and both build profiles; the accelerated build used to
-- answer no rows, reading the index at a snapshot without its entries.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_rd;
CREATE TABLE ibi_rd (a INT);
INSERT INTO ibi_rd VALUES (1);
BEGIN;
CREATE INDEX ibi_rd_i ON ibi_rd (a);

-- @test
SELECT a FROM ibi_rd WHERE a = 1;
