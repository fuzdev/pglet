-- kysely 0.28.8's `PostgresIntrospector.getTables` query, which its `Migrator` runs before
-- it migrates: `col_description`, `pg_get_serial_sequence` over `quote_ident`-built names,
-- `has_schema_privilege`, and `nspname !~ $1` with the pattern bound as a parameter
-- (`'^pg_'`, as kysely binds it). kysely binds its other values too; here the `relkind`
-- list, the `attnum` bound and the `name` comparisons are literals — bound, they take
-- `"char"`, `smallint` and `name`, which the harness's parameter bindings do not spell, and
-- a binary-format `name` parameter is `42704` on pglet (no `name` receive arm). One condition
-- is added to kysely's text, the last one: the corpus shares a server, and the relations
-- other fixtures leave behind would otherwise be listed too.
-- @setup
CREATE TABLE kt (id serial PRIMARY KEY, ident int GENERATED ALWAYS AS IDENTITY, plain int NOT NULL DEFAULT 3, "Mixed" text, big bigserial);
CREATE TABLE "select" ("my col" serial);
-- @param $1 text ^pg_
-- @param $2 bool true
-- @test
select "a"."attname" as "column", "a"."attnotnull" as "not_null", "a"."atthasdef" as "has_default", "c"."relname" as "table", "c"."relkind" as "table_type", "ns"."nspname" as "schema", "typ"."typname" as "type", "dtns"."nspname" as "type_schema", col_description(a.attrelid, a.attnum) as "column_description", pg_get_serial_sequence(quote_ident(ns.nspname) || '.' || quote_ident(c.relname), a.attname) as "auto_incrementing" from "pg_catalog"."pg_attribute" as "a" inner join "pg_catalog"."pg_class" as "c" on "a"."attrelid" = "c"."oid" inner join "pg_catalog"."pg_namespace" as "ns" on "c"."relnamespace" = "ns"."oid" inner join "pg_catalog"."pg_type" as "typ" on "a"."atttypid" = "typ"."oid" inner join "pg_catalog"."pg_namespace" as "dtns" on "typ"."typnamespace" = "dtns"."oid" where "c"."relkind" in ('r', 'v', 'p') and "ns"."nspname" !~ $1 and "ns"."nspname" != 'information_schema' and "ns"."nspname" != 'crdb_internal' and has_schema_privilege(ns.nspname, 'USAGE') and "a"."attnum" >= 0 and "a"."attisdropped" != $2 and "c"."relname" != 'kysely_migration' and "c"."relname" != 'kysely_migration_lock' and "c"."relname" in ('kt', 'select') order by "ns"."nspname", "c"."relname", "a"."attnum";
