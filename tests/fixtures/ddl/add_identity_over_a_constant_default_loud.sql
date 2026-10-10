-- The identity clause over a column that has a default is refused: an identity column's
-- value is its sequence's, and the clause does not drop a default the column was given.
-- @expect-error: the column already has a default (55000)
-- @expect-message
-- @setup
CREATE TABLE aid (c INT NOT NULL DEFAULT 5, v TEXT);
-- @test
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
