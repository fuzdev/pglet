-- A default given by `SET DEFAULT` after the table is one the identity clause refuses.
-- @expect-error: the column already has a default (55000)
-- @expect-message
-- @setup
CREATE TABLE aid (c SMALLINT NOT NULL, v TEXT);
ALTER TABLE aid ALTER COLUMN c SET DEFAULT 3;
-- @test
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
