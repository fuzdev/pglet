-- The identity clause over a table no relation names is the relation's refusal.
-- @expect-error: no such relation (42P01)
-- @expect-message
-- @setup
CREATE TABLE ai (c INT, v TEXT);
-- @test
ALTER TABLE ai_nosuch ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
