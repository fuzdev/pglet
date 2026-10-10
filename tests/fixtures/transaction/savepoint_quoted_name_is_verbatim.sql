-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper.
--
-- The other half of `savepoint_unquoted_name_is_downcased.sql`: a **quoted** name
-- is taken exactly, so `SAVEPOINT "MySp"` does not answer to the folded `mysp`.
-- Its setup cannot fail — a `BEGIN` and a `SAVEPOINT` — so the `3B001` it records
-- is provably the `@test`'s.
-- @setup
BEGIN;
SAVEPOINT "MySp";

-- @expect-error: the quoted name is verbatim, so the folded `mysp` does not exist (3B001)
-- @test
ROLLBACK TO mysp;
