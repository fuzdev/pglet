-- The minted sequence name is an enum's, over a column that has a default: the type
-- namespace's refusal wins.
-- @expect-error: type already exists (42710)
-- @expect-message
-- @setup
CREATE TYPE aid_c_seq AS ENUM ('x');
CREATE TABLE aid (c INT NOT NULL DEFAULT 5, v TEXT);
-- @test
ALTER TABLE aid ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
