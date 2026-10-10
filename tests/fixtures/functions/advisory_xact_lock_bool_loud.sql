-- A `boolean` key names no overload: `42883`.
-- @test
-- @expect-error: pg_advisory_xact_lock(boolean) does not exist
-- @expect-message
SELECT pg_advisory_xact_lock(true);
