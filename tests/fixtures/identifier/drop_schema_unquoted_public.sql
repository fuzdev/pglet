-- The accept side of `drop_schema_quoted_public_loud.sql`: an unquoted `PUBLIC` folds
-- and still means the implicit schema, so the reset idiom keeps working in upper case.
-- Without this polarity a repair that made the schema test exact *without* folding the
-- identifier would ship green, and every caller spelling the idiom in upper case would
-- get `3F000` instead of a reset.
-- The observable is the table count after the drop, not the command tag: a `DROP
-- SCHEMA` that reported success without clearing anything would pass a tag-only check.
-- @setup
CREATE TABLE dsu_old (a INT);
INSERT INTO dsu_old VALUES (1);
DROP SCHEMA PUBLIC CASCADE;
CREATE SCHEMA public;
CREATE TABLE dsu_new (a INT);
INSERT INTO dsu_new VALUES (2);

-- @test
SELECT table_name::text AS table_name
FROM information_schema.tables
WHERE table_schema = 'public'
ORDER BY table_name;
