-- @no-txn
-- Runs with no transaction open, so it opts out of the harness's isolating
-- wrapper.
--
-- The `RELEASE` member of the `25P01` family.
-- @setup
SELECT 1;

-- @expect-error: RELEASE SAVEPOINT can only be used in transaction blocks (25P01)
-- @test
RELEASE SAVEPOINT s;
