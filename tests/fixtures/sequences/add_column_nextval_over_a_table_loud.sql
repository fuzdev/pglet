-- ...and on the `ADD COLUMN` route.
-- @setup
CREATE TABLE acnt_host (a INT);
CREATE TABLE acnt_user (a INT);
-- @test
ALTER TABLE acnt_user ADD COLUMN b INT DEFAULT nextval('acnt_host');
-- @expect-error: pglet-stricter; PG defers this to the first INSERT (42809)
