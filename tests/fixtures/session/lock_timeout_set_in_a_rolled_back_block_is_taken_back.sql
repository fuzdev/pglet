-- @no-txn
-- @setup
SET lock_timeout = '3s';
-- Drives its own BEGIN/ROLLBACK, so it opts out of the harness's isolating wrapper.
--
-- `lock_timeout` is transactional, as every 18.6 GUC is (`AtEOXact_GUC`): a `SET` inside
-- a block the block then rolls back is taken back with it. pglet acts on the value — it
-- bounds a statement's wait for the writer — so a rolled-back `SET lock_timeout = 0` that
-- stood would leave every later wait unbounded. The session value is set first because
-- the two engines' defaults differ (pglet's `1s`, 18.6's `0`), and ahead of this comment
-- because a comment-prefixed `SET` is the native server's `0A000` (`EDGES.md` row 23).
BEGIN;
SET lock_timeout = '7s';
ROLLBACK;
-- @test
SHOW lock_timeout;
