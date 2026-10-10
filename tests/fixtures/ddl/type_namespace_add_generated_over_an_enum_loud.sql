-- `ALTER COLUMN … ADD GENERATED` mints `<table>_<column>_seq` through the same gate as
-- `CREATE TABLE`, so over an enum of that name it is `42710` too.
-- @setup
CREATE TYPE tnt9_a_seq AS ENUM ('x');
CREATE TABLE tnt9 (a INT NOT NULL);
-- @expect-error: the identity sequence over an enum
-- @expect-message
-- @test
ALTER TABLE tnt9 ALTER COLUMN a ADD GENERATED ALWAYS AS IDENTITY;
