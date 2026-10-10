-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper.
--
-- `RELEASE` takes only the optional `SAVEPOINT` noise word — there is no
-- `RELEASE TRANSACTION` production in `gram.y`, unlike `ROLLBACK`, which takes
-- `opt_transaction` before its `TO`. The asymmetry is the reason to pin it: a
-- parser that shared one noise-word helper between the two would accept this.
-- @setup
BEGIN;
SAVEPOINT s;

-- @expect-error: no RELEASE TRANSACTION production (42601)
-- @test
RELEASE TRANSACTION s;
