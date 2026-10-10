-- `ADD GENERATED … AS IDENTITY` over a column not declared `NOT NULL` is refused: an
-- identity column is `NOT NULL`, and the clause reads no row, so it may not be what first
-- declares it. Here a row holds a NULL.
-- @expect-error: the column is not declared NOT NULL
-- @expect-message
-- @setup
CREATE TABLE ai (c INT, v TEXT);
INSERT INTO ai VALUES (NULL, 'null'), (1, 'one');
-- @test
ALTER TABLE ai ALTER COLUMN c ADD GENERATED ALWAYS AS IDENTITY;
