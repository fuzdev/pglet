-- An UNTAKEN CASE branch is still validated at plan time: `WHEN 1=1` takes the THEN,
-- but the `ELSE 'x'` (unified to int) is an unparseable constant cast and errors
-- (`22P02`), matching Postgres's parse-time constant folding. Previously pglet
-- evaluated only the taken branch and silently returned `1`.
-- @test
-- @expect-error: unparseable untaken ELSE branch errors at plan time (22P02)
SELECT CASE WHEN 1=1 THEN 1 ELSE 'x' END;
