-- The `ADD COLUMN` route of the same cook.
-- @setup
CREATE TYPE tnt17s AS ENUM ('x');
CREATE TABLE tnt17 (a INT);
-- @expect-error: nextval of an enum's name, ADD COLUMN
-- @expect-message
-- @test
ALTER TABLE tnt17 ADD COLUMN b INT DEFAULT nextval('tnt17s') + 1;
