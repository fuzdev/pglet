-- Three arguments name no overload: `42883` on both engines.
-- @test
-- @expect-error: pg_advisory_xact_lock_shared(integer, integer, integer) does not exist
SELECT pg_advisory_xact_lock_shared(1, 2, 3);
