-- `42710` from a second constraint of one name under `IF EXISTS`.
-- @setup
CREATE TABLE atie13 (a INT);
ALTER TABLE IF EXISTS atie13 ADD CONSTRAINT atie13_ck CHECK (a > 0);
-- @expect-error: the constraint name is taken
-- @test
ALTER TABLE IF EXISTS atie13 ADD CONSTRAINT atie13_ck CHECK (a > 1);
