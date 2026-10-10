-- `current_schemas` is not a keyword on 18.6, so the bare word is an ordinary
-- column reference and `42703` — the polarity that keeps the new special-form set
-- from over-reaching. Its siblings `version`, `current_database`, `current_setting`
-- and `pg_typeof` behave the same way.
-- @expect-error: bare current_schemas is a column, not the function
-- @test
SELECT current_schemas;
