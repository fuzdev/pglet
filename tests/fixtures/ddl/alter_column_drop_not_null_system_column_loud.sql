-- A system column is not the relation's to alter: `0A000`.
-- @expect-error: system column
-- @setup
CREATE TABLE dns (v int NOT NULL);
-- @test
ALTER TABLE dns ALTER COLUMN ctid DROP NOT NULL;
