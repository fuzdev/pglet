-- An action's own error is raised under `IF EXISTS` as without it: `42701`.
-- @setup
CREATE TABLE atie9 (a INT);
-- @expect-error: the column already exists
-- @expect-message
-- @test
ALTER TABLE IF EXISTS atie9 ADD COLUMN a INT;
