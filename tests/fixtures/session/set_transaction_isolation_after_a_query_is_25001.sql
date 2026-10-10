-- The isolation level cannot change once the block has run a query — 18.6's
-- `check_transaction_isolation`, `25001 SET TRANSACTION ISOLATION LEVEL must be called
-- before any query`. The harness's own isolating `BEGIN … ROLLBACK` is the block, and the
-- setup's `SELECT` is its first query.
-- @setup
SELECT 1;
-- @test
-- @expect-error: SET TRANSACTION ISOLATION LEVEL must be called before any query
SET TRANSACTION ISOLATION LEVEL SERIALIZABLE;
