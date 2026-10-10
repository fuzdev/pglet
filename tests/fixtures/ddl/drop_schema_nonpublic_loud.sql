-- pglet models a single implicit `public` schema, so any other schema name does
-- not exist: `DROP SCHEMA <other>` fails loud (3F000, invalid_schema_name),
-- matching Postgres on a missing schema.
-- @expect-error: schema does not exist
-- @test
DROP SCHEMA does_not_exist;
