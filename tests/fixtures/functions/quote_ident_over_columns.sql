-- `quote_ident` over catalog columns of type `name`, the way kysely builds a qualified
-- name for `pg_get_serial_sequence`, and over a `varchar`.
-- @setup
CREATE TABLE "Qi Table" (v varchar);
INSERT INTO "Qi Table" VALUES ('select'), ('ok');
-- @test
SELECT quote_ident(v) AS qv, (SELECT quote_ident(ns.nspname) || '.' || quote_ident(c.relname)
  FROM pg_catalog.pg_class c JOIN pg_catalog.pg_namespace ns ON c.relnamespace = ns.oid
  WHERE c.relname = 'Qi Table') AS qualified
FROM "Qi Table" ORDER BY v;
