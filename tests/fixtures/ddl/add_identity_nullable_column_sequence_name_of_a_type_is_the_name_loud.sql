-- The minted sequence name `<table>_<column>_seq` is an enum's, over a column not
-- declared `NOT NULL`: the type namespace's refusal wins over the declaration's.
-- @expect-error: type already exists (42710)
-- @expect-message
-- @setup
CREATE TYPE ai_c_seq AS ENUM ('x');
CREATE TABLE ai (c INT, v TEXT);
-- @test
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
