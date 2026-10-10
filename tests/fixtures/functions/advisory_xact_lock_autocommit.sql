-- Outside a block the lock is the statement's: taken and released by the one
-- `SELECT`. The two-connection release is `test_concurrent`'s; this pins the answer.
-- @no-txn
-- @test
SELECT pg_advisory_xact_lock(7) AS k;
