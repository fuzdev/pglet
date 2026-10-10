-- A column with a default that is not declared `NOT NULL`: the declaration's refusal is
-- the one raised, ahead of anything said about the default.
-- @expect-error: the column is not declared NOT NULL
-- @expect-message
-- @setup
CREATE TABLE ai (c INT DEFAULT 5, v TEXT);
-- @test
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
