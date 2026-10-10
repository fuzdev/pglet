-- `CURRENT_CATALOG` (a keyword, no `pg_proc` entry) and `current_database()` (a
-- function, no keyword) name one fact under two spellings — and two output column
-- names, which is why they are two `BuiltinFn` variants.
-- @test
SELECT current_catalog, current_database();
