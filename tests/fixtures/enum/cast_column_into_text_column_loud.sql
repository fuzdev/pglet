-- A column's value converted into a text column: the row carrying `q` is refused, where
-- the label landed.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE e (s TEXT);
INSERT INTO e VALUES ('q');
CREATE TABLE tx (v TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
INSERT INTO tx SELECT s::mood FROM e;
