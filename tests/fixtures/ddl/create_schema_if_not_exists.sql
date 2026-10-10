-- `CREATE SCHEMA IF NOT EXISTS public` is a no-op success on both engines (the
-- public schema already exists), so the schema stays usable. pglet models a
-- single always-present implicit `public`; this is the differentiable CREATE
-- SCHEMA path (a bare `CREATE SCHEMA public` diverges — Postgres raises 42P06).
-- @setup
CREATE SCHEMA IF NOT EXISTS public;
CREATE TABLE t (id INT, label TEXT);
INSERT INTO t VALUES (42, 'ok');
-- @test
SELECT id, label FROM t;
