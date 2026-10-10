-- @no-txn
-- Drives its own transactions, so it opts out of the harness's isolating wrapper.
--
-- `ROLLBACK TO` destroys every savepoint established **after** its target while
-- keeping the target itself — so the inner name is gone and naming it is `3B001`.
-- The pair with `savepoint_rollback_to_is_repeatable.sql` is what fixes the rule:
-- that one shows the target survives, this one that its juniors do not.
--
-- The `RELEASE` polarity is a separate fixture and not a duplicate of this one: a
-- release pops its own level as part of its job, so an implementation that forgot
-- to drop the *inner* levels still answers the release case correctly by accident
-- (measured as a mutation) and only this spelling catches it.
-- @setup
BEGIN;
SAVEPOINT outer_sp;
SAVEPOINT inner_sp;
ROLLBACK TO outer_sp;

-- @expect-error: the inner savepoint was destroyed by the rollback to the outer one
-- @test
ROLLBACK TO inner_sp;
