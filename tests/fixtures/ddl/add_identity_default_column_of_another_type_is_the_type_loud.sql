-- A column that has a default and is no integer: the type's refusal wins, the identity's
-- sequence — where the type is judged — being created before the column's default is read.
-- @expect-error: identity column type (22023)
-- @expect-message
-- @setup
CREATE TABLE aid (c TEXT NOT NULL DEFAULT 'x', v TEXT);
-- @test
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
