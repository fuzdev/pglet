-- `IF EXISTS` skips a missing table and nothing else: a default refuses the clause.
-- @expect-error: the column already has a default (55000)
-- @expect-message
-- @setup
CREATE TABLE aid (c INT NOT NULL DEFAULT 5, v TEXT);
-- @test
ALTER TABLE IF EXISTS aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
