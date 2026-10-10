-- @setup
SET TRANSACTION ISOLATION LEVEL REPEATABLE READ;
-- `SET TRANSACTION ISOLATION LEVEL` as the block's first statement sets the block's level,
-- and `SHOW transaction_isolation` reports it. The harness's own isolating `BEGIN …
-- ROLLBACK` is the block; with the level not carried, `SHOW` answers `read committed`.
-- This comment follows the statement because a comment-prefixed `SET` or `SHOW` is the
-- native server's `0A000` (`EDGES.md` row 23).
-- @test
SHOW transaction_isolation;
