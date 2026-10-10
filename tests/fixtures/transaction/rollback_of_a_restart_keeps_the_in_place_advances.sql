-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper
-- (and keeps its own DROP-first cleanup).
--
-- The third value a rollback can restore a sequence to, and the one neither
-- neighbouring fixture reaches. `ALTER SEQUENCE … RESTART` gives the sequence a
-- **new** counter cell, so rolling back puts the old cell back — but the old cell
-- is not what it held at `BEGIN`: the two `nextval`s before the restart wrote it
-- **in place**, non-transactionally. So the answer after the rollback is `3`, not
-- `1` (the `BEGIN` state) and not `78` (the restarted cell).
--
-- `sequences/alter_restart_survives_rollback.sql` cannot see this: its `nextval`
-- runs before the `BEGIN`, so the snapshot's value and the outgoing cell's value
-- coincide and either rule passes. Measured on 18.6.
-- @setup
DROP SEQUENCE IF EXISTS rst_inplace CASCADE;
CREATE SEQUENCE rst_inplace;
BEGIN;
SELECT nextval('rst_inplace');
SELECT nextval('rst_inplace');
ALTER SEQUENCE rst_inplace RESTART WITH 77;
SELECT nextval('rst_inplace');
ROLLBACK;

-- @test
SELECT nextval('rst_inplace') AS after_rollback;
