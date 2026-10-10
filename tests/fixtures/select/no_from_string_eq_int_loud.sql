-- A no-FROM comparison of an unparseable string against an int literal coerces the
-- string to int and fails loud (`22P02`) — Postgres resolves the unknown literal to
-- int. Previously pglet text-compared and silently returned `f`.
-- @test
-- @expect-error: unparseable string vs int literal errors 22P02
SELECT 'abc' = 1;
