-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating wrapper.
--
-- A RESTART is transactional: rolled back, the counter continues from where it was.
-- pglet gets this from `Plan::swapped_sequences`, which records the counter the RESTART
-- replaces before it runs, so the rollback puts that value back rather than carrying
-- the restarted counter forward.
-- @setup
CREATE SEQUENCE rst_rollback START WITH 5;
SELECT nextval('rst_rollback');
BEGIN;
ALTER SEQUENCE rst_rollback RESTART WITH 77;
ROLLBACK;
-- @test
SELECT nextval('rst_rollback');
