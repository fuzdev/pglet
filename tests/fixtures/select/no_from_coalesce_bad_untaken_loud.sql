-- COALESCE validates every branch eagerly: `COALESCE(NULL, 1, 'x')` returns the `1`,
-- but the later `'x'` (unified to int) is an unparseable constant cast and errors
-- (`22P02`), matching Postgres. Previously pglet returned `1` silently.
-- @test
-- @expect-error: unparseable untaken COALESCE branch errors at plan time (22P02)
SELECT COALESCE(NULL, 1, 'x');
