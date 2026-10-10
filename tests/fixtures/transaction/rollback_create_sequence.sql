-- @no-txn
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating
-- transaction wrapper.
--
-- A catalog change that does *not* invalidate a cached query plan is still a
-- catalog change ROLLBACK has to undo. `CREATE SEQUENCE` is the one such
-- statement (a sequence never appears in a plan, so it deliberately leaves
-- `Schema::generation` alone), and when it is **alone** in its transaction there
-- is no other DDL to carry the restore: a rollback gated on a generation change
-- would conclude the transaction had changed nothing and skip the catalog restore,
-- so the sequence would outlive its own ROLLBACK and `nextval` return 1. Pairing it
-- with any generation-bumping DDL would hide that, which is why this fixture keeps
-- the transaction to exactly one statement.
-- @setup
BEGIN;
CREATE SEQUENCE rollback_seq;
ROLLBACK;

-- @expect-error: the sequence did not survive its rollback
-- @test
SELECT nextval('rollback_seq');
