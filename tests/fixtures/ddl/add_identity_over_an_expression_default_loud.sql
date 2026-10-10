-- An expression default is a default the identity clause refuses.
-- @expect-error: the column already has a default (55000)
-- @expect-message
-- @setup
CREATE TABLE aid (c INT NOT NULL DEFAULT 1 + 1, v TEXT);
-- @test
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
