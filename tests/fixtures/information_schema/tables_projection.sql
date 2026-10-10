-- information_schema.tables — one row per user table (the spine's
-- auth_tables_exist migration probe: `EXISTS (SELECT 1 FROM
-- information_schema.tables WHERE table_schema='public' AND table_name=$1)`).
-- Filtered to a known table so the result is stable vs Postgres (which also lists
-- system catalogs); table_catalog is excluded (PG uses the database name).
-- @no-declared-types: all three projected columns diverge — 18.6 sends `name` (19) for
--   `table_schema` and `table_name` and `varchar` (1043) for `table_type`, where pglet
--   sends `text` (25) for each. Both of those OIDs exist here (`TypeOid::Name`,
--   `TypeOid::Varchar`); what pglet has no model of is a **per-column declared type for
--   a catalog view**, so these three report `text`. Same rendered values;
--   giving the catalog views their own column types is its own change.
-- @setup
CREATE TABLE widget (id INT PRIMARY KEY, name TEXT);
-- @test
SELECT table_schema, table_name, table_type
FROM information_schema.tables
WHERE table_name = 'widget';
