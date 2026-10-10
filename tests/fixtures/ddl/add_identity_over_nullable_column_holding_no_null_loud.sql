-- The refusal reads the column's declaration and not its rows: no row holds a NULL here,
-- and the clause is refused all the same.
-- @expect-error: the column is not declared NOT NULL
-- @expect-message
-- @setup
CREATE TABLE ai (c INT, v TEXT);
INSERT INTO ai VALUES (1, 'one'), (2, 'two');
-- @test
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
