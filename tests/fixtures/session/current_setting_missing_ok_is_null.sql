-- `missing_ok = true` turns the unrecognized-parameter error into NULL; that is the
-- only thing the second argument does.
-- @test
SELECT current_setting('nosuchthing', true) AS missing;
