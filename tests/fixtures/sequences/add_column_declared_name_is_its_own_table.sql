-- The `ADD COLUMN` twin. Same reason, and it needs its own fixture because the two
-- routes call the shared gate from different code and could drift.
-- @setup
CREATE TABLE acown (x INT);
-- @test
ALTER TABLE acown ADD COLUMN y INT GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME acown);
-- @expect-error: relation already exists (42P07)
