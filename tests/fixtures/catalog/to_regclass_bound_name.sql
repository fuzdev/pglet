-- The spine's existence probe, bound: `to_regclass($1) IS NOT NULL` before a migration has
-- created the table and after, with the parameter typed `text` from its position.
-- @setup
CREATE TABLE schema_version (version INT);
-- @param $1 text public.schema_version
-- @param $2 text fact_holder
-- @test
SELECT to_regclass($1) IS NOT NULL AS migrated, to_regclass($2) IS NOT NULL AS holder,
       to_regclass($1)::text AS name;
