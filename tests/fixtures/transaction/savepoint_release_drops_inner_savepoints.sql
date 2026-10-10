-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper.
--
-- The `RELEASE` polarity of `savepoint_rollback_to_drops_inner_savepoints.sql`:
-- releasing an outer savepoint drops the inner ones with it, so naming the inner
-- one afterwards is `3B001`. Keeping the writes and keeping the *savepoints* are
-- different questions, and `RELEASE` answers them differently.
-- @setup
DROP TABLE IF EXISTS sp_relinner CASCADE;
CREATE TABLE sp_relinner (id INT);
BEGIN;
SAVEPOINT a;
SAVEPOINT b;
RELEASE a;

-- @expect-error: the inner savepoint was released with the outer one
-- @test
RELEASE b;
