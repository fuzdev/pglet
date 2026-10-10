-- A positional set-op `ORDER BY n` whose ordinal is past the output-column count fails
-- loud (42P10, Postgres: "ORDER BY position 5 is not in select list") rather than the
-- old 42601 syntax error.
-- @expect-error: set-op ORDER BY position out of range (42P10)
-- @test
SELECT 1 UNION SELECT 2 ORDER BY 5;
