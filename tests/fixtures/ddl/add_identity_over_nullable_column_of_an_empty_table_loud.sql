-- The refusal reads the column's declaration and not its rows: the table holds none.
-- @expect-error: the column is not declared NOT NULL
-- @expect-message
-- @setup
CREATE TABLE ai (c INT, v TEXT);
-- @test
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
