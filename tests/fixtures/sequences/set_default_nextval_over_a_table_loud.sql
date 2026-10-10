-- ...the same gate on the `SET DEFAULT` route (the pg_dump-restore path), so the
-- three DDL registration sites cannot drift apart.
-- @setup
CREATE TABLE sdnt_host (a INT);
CREATE TABLE sdnt_user (b INT);
-- @test
ALTER TABLE sdnt_user ALTER COLUMN b SET DEFAULT nextval('sdnt_host');
-- @expect-error: pglet-stricter; PG defers this to the first INSERT (42809)
