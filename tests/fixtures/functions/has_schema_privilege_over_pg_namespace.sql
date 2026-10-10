-- The shape kysely's introspector issues: the schema argument is `pg_namespace.nspname`,
-- a `name`.
-- @test
SELECT nspname, has_schema_privilege(nspname, 'USAGE') AS usable
FROM pg_catalog.pg_namespace WHERE nspname IN ('public', 'pg_catalog') ORDER BY nspname;
