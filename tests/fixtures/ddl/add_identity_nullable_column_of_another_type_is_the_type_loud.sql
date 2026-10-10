-- A column that is neither an integer nor declared `NOT NULL`: the type's refusal wins.
-- 18.6 creates the identity's sequence, which is where the type is judged, before it
-- looks at the column's declaration.
-- @expect-error: identity column type (22023)
-- @expect-message
-- @setup
CREATE TABLE ai (c TEXT, v TEXT);
-- @test
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
