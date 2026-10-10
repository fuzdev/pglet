-- A no-FROM CASE whose branches type as int (the THEN) but whose ELSE is an
-- unparseable string coerces the ELSE to int and fails loud (`22P02`) — Postgres
-- unifies the branch types to int. Previously the no-FROM path skipped branch
-- unification and returned the text `'x'`.
-- @test
-- @expect-error: unparseable ELSE string unified to int errors 22P02
SELECT CASE WHEN false THEN 1 ELSE 'x' END;
