-- A column the table does not have is `42703`.
-- @expect-error: no such column
-- @expect-message
-- @setup
CREATE TABLE dnm (v int NOT NULL);
-- @test
ALTER TABLE dnm ALTER COLUMN nope DROP NOT NULL;
