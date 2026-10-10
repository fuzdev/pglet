-- `IF EXISTS` skips a missing table and nothing else: over a table that exists, a column
-- not declared `NOT NULL` refuses the identity clause.
-- @expect-error: the column is not declared NOT NULL
-- @expect-message
-- @setup
CREATE TABLE ai (c INT, v TEXT);
-- @test
ALTER TABLE IF EXISTS ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
