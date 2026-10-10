-- The identity clause over a column the table lacks is the column's refusal.
-- @expect-error: no such column (42703)
-- @expect-message
-- @setup
CREATE TABLE ai (c INT, v TEXT);
-- @test
ALTER TABLE ai ALTER COLUMN nosuch ADD GENERATED ALWAYS AS IDENTITY;
