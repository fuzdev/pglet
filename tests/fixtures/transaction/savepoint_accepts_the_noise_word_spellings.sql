-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper.
--
-- The grammar's optional words, all four in one block: `ROLLBACK WORK TO
-- SAVEPOINT`, `ROLLBACK TRANSACTION TO`, `ROLLBACK TO SAVEPOINT` and
-- `RELEASE SAVEPOINT`. `gram.y` puts the `TO` tail **after** `opt_transaction`,
-- which is not guessable from the bare spellings, and there is no
-- `RELEASE TRANSACTION` at all (`release_transaction_is_a_syntax_error.sql`).
-- @setup
BEGIN;
SAVEPOINT s;
ROLLBACK WORK TO SAVEPOINT s;
ROLLBACK TRANSACTION TO s;
ROLLBACK TO SAVEPOINT s;
RELEASE SAVEPOINT s;
COMMIT;

-- @test
SELECT 1 AS ok;
