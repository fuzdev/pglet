-- `current_schemas(include_implicit)` — the only `name[]` (OID 1003) producer, so
-- this is also the array-of-`name` wire encoding's guard on both formats.
--
-- `true` prepends `pg_catalog`, which Postgres searches whether or not the path
-- names it.
-- @test
SELECT current_schemas(true) AS implicit, current_schemas(false) AS just_path;
