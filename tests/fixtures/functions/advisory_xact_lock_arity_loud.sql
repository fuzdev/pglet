-- Three arguments name no overload: `42883` on both engines.
-- @test
-- @expect-error: pg_advisory_xact_lock(integer, integer, integer) does not exist
SELECT pg_advisory_xact_lock(1, 2, 3);
