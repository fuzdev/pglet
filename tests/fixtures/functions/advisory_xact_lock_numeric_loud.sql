-- A `numeric` key names no overload: `42883`. A literal past `bigint` is `numeric` too.
-- @test
-- @expect-error: pg_advisory_xact_lock(numeric) does not exist
-- @expect-message
SELECT pg_advisory_xact_lock(9223372036854775808);
