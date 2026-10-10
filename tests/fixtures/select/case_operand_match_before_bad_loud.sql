-- A simple `CASE x WHEN v` is Postgres's `x = v`, resolved for **every** arm at parse
-- time — so a mistyped later arm errors even when an earlier arm matches. pglet
-- evaluated the arms in order and returned `'one'`, exactly the short-circuit the
-- `IN`-list precedent (`types/compare_in_list_match_before_bad_loud.sql`) already
-- rules out for the unparseable-literal case.
-- @test
-- @expect-error: a mistyped WHEN arm after a matching one still errors the query
SELECT CASE 1 WHEN 1 THEN 'one' WHEN true THEN 'two' END;
