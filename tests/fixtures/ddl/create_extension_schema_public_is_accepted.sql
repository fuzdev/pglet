-- The accepting side of `create_extension_schema_must_exist.sql` — `public` is the
-- one schema pglet models, and it is the schema every migration names here. Without
-- this pair the `3F000` could be a blanket refusal of the option.
-- @test
CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
SELECT gen_random_uuid() IS NOT NULL AS ok;
