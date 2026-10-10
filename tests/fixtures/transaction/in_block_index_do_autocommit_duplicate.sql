-- @no-txn
-- Runs its DO as an autocommit statement, so it opts out of the harness's isolating
-- wrapper (and keeps its own DROP-first cleanup).
--
-- A DO block with no transaction open builds a unique index and then writes the committed
-- row's key: 23505 on both engines. The statement's own storage transaction reads the
-- build as an explicit block does, and used to miss it the same way.
-- @profile-lock
-- @setup
DROP TABLE IF EXISTS ibi_doa;
CREATE TABLE ibi_doa (a INT);
INSERT INTO ibi_doa VALUES (1);

-- @expect-error: the committed row holds the key
-- @test
DO $$ BEGIN CREATE UNIQUE INDEX ibi_doa_u ON ibi_doa (a); INSERT INTO ibi_doa VALUES (1); END $$;
