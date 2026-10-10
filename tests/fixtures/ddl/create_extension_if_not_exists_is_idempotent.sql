-- The line every migration that touches UUIDs opens with, and its repeat. pglet
-- models no extension catalog — `uuid_generate_v4()` is a builtin here, with or
-- without the statement — so the statement's only job is to be accepted, twice, and
-- to leave the function resolving. The final `SELECT` is what makes this an
-- assertion rather than a pair of empty results.
-- @test
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS "uuid-ossp";
CREATE EXTENSION IF NOT EXISTS pgcrypto WITH SCHEMA public;
SELECT uuid_generate_v4() IS NOT NULL AS uuid_ossp, gen_random_uuid() IS NOT NULL AS pgcrypto;
