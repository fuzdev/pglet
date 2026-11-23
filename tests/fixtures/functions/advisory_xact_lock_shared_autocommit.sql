-- Outside a block the shared lock is the statement's, as the exclusive one is.
-- @no-txn
-- @test
SELECT pg_advisory_xact_lock_shared(7) AS k;
