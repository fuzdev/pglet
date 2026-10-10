-- The `SET DEFAULT` route of the same cook.
-- @setup
CREATE TYPE tnt18s AS ENUM ('x');
CREATE TABLE tnt18 (a INT);
-- @expect-error: nextval of an enum's name, SET DEFAULT
-- @expect-message
-- @test
ALTER TABLE tnt18 ALTER COLUMN a SET DEFAULT nextval('tnt18s');
