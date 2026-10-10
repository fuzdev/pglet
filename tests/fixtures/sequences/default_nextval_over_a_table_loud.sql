-- A user's `DEFAULT nextval('x')` over a **table** name. pglet is deliberately
-- stricter than Postgres here and the divergence is one of *timing*, not verdict:
-- Postgres resolves `'x'::regclass` at parse analysis and checks the kind only when
-- the sequence is first read, so it accepts this DDL and answers 42809 on every
-- INSERT. pglet has no regclass and auto-creates the named sequence at DDL time, so
-- accepting would put one name in both `tables` and `sequences` — the state the
-- relation namespace declares impossible, and one whose dump emits a CREATE SEQUENCE
-- and a CREATE TABLE under one name that neither engine reloads.
-- @setup
CREATE TABLE dnt_host (a INT);
-- @test
CREATE TABLE dnt_user (b INT DEFAULT nextval('dnt_host'));
-- @expect-error: pglet-stricter; PG defers this to the first INSERT (42809)
