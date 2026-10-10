-- @no-txn
-- Runs with no transaction open, which is the whole point, so it opts out of the
-- harness's isolating wrapper — inside one the statement would succeed.
--
-- Postgres's `CheckTransactionBlock`: `SAVEPOINT` outside a block is `25P01`,
-- not a no-op and not a syntax error.
-- @setup
SELECT 1;

-- @expect-error: SAVEPOINT can only be used in transaction blocks (25P01)
-- @test
SAVEPOINT s;
