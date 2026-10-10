-- `(int4, int4)` takes no `int8`: `42883`.
-- @test
-- @expect-error: pg_advisory_xact_lock(bigint, integer) does not exist
-- @expect-message
SELECT pg_advisory_xact_lock(1::bigint, 2);
