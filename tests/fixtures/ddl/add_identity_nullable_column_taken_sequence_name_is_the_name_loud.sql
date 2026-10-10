-- A declared `SEQUENCE NAME` a relation already holds, over a column not declared
-- `NOT NULL`: the name's refusal wins, the sequence being created before the column's
-- declaration is read.
-- @expect-error: relation already exists (42P07)
-- @expect-message
-- @setup
CREATE TABLE ai (c INT, v TEXT);
CREATE TABLE ai_taken (x INT);
-- @test
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY (SEQUENCE NAME ai_taken);
