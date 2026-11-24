-- An identity column is `NOT NULL` by definition: `42601`, with `SET DEFAULT`'s sentence.
-- @expect-error: identity column
-- @expect-message
-- @setup
CREATE TABLE dni (id int GENERATED ALWAYS AS IDENTITY, v int);
-- @test
ALTER TABLE dni ALTER COLUMN id DROP NOT NULL;
