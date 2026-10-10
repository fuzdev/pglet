-- `current_schema` *is* callable (`pg_proc` 1402), so a `(` here reaches signature
-- resolution rather than the scanner — and one argument resolves nothing.
-- @expect-error: no current_schema(integer) signature
-- @test
SELECT current_schema(1);
