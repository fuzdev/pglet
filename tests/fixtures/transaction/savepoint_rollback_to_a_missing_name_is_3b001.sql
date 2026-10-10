-- @no-txn
-- Drives its own transaction, so it opts out of the harness's isolating wrapper.
--
-- Inside a block a name no live savepoint holds is `3B001`, not `25P01` — the
-- block check runs first and passes. (That it also **aborts** the block is the
-- separate claim `savepoint_rollback_to_a_missing_name_aborts_the_block.sql`
-- pins, since an `@expect-error` fixture cannot see what happens next.)
-- @setup
BEGIN;
SELECT 1;

-- @expect-error: savepoint "nope" does not exist (3B001)
-- @test
ROLLBACK TO nope;
