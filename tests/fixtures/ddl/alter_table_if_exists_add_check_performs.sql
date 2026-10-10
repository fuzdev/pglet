-- `ADD CONSTRAINT … CHECK` under `IF EXISTS` is enforced.
-- @setup
CREATE TABLE atie5 (a INT);
ALTER TABLE IF EXISTS atie5 ADD CONSTRAINT atie5_pos CHECK (a > 0);
-- @expect-error: the check the IF EXISTS form added
-- @expect-message
-- @test
INSERT INTO atie5 VALUES (-1);
