-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper.
--
-- The `RELEASE` member of the `3B001` family — a separate route from
-- `ROLLBACK TO`, which the aborted-block gate treats differently.
-- @setup
BEGIN;
SELECT 1;

-- @expect-error: savepoint "nope" does not exist (3B001)
-- @test
RELEASE nope;
