-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- A two-column unique index built inside the block holds the committed rows' key pairs:
-- a new pair lands and a committed pair is 23505, on both engines.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_mc;
CREATE TABLE ibi_mc (a INT, b INT);
INSERT INTO ibi_mc VALUES (1, 1), (1, 2);
BEGIN;
CREATE UNIQUE INDEX ibi_mc_u ON ibi_mc (a, b);
INSERT INTO ibi_mc VALUES (1, 3);

-- @expect-error: the committed pair holds the key
-- @test
INSERT INTO ibi_mc VALUES (1, 1);
