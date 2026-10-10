-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper (and
-- keeps its own DROP-first cleanup): the rows have to be committed before the block.
--
-- A DO inside an open block builds a unique index and then writes the committed row's key:
-- 23505 on both engines.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_dob;
CREATE TABLE ibi_dob (a INT);
INSERT INTO ibi_dob VALUES (1);
BEGIN;

-- @expect-error: the committed row holds the key
-- @test
DO $$ BEGIN CREATE UNIQUE INDEX ibi_dob_u ON ibi_dob (a); INSERT INTO ibi_dob VALUES (1); END $$;
