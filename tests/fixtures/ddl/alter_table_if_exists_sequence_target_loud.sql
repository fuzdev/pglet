-- Only a name **no relation** holds skips: over a sequence the action runs and meets
-- the wrong-kind `42809` it meets without `IF EXISTS`.
-- @setup
CREATE SEQUENCE atie14;
-- @expect-error: a sequence is not a table
-- @test
ALTER TABLE IF EXISTS atie14 ADD COLUMN x INT;
