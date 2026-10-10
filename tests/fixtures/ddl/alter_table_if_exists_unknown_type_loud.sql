-- `42704` under `IF EXISTS`, over a table that exists.
-- @setup
CREATE TABLE atie11 (a INT);
-- @expect-error: no such type
-- @test
ALTER TABLE IF EXISTS atie11 ADD COLUMN b nosuchtype_atie11;
