-- A narrowing cast whose value is out of the target's range fails loud (Postgres
-- "integer out of range", SQLSTATE 22003) rather than silently wrapping via an
-- `as` cast. `9999999999` is a bigint; narrowing it to int4 overflows.
-- @test
-- @expect-error: integer out of range
SELECT (9999999999::int8)::int4;
