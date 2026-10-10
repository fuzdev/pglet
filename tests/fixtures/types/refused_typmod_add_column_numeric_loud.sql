-- `numeric`'s half of `ADD COLUMN`.
-- @setup
CREATE TABLE rtan (a INT);
-- @expect-error: a NUMERIC precision above 1000 at ADD COLUMN
-- @expect-message
-- @test
ALTER TABLE rtan ADD COLUMN c NUMERIC(1001);
