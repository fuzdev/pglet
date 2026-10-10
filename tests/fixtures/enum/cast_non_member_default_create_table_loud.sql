-- A column `DEFAULT` holding the cast is refused at the DDL, over zero rows: the default is
-- cooked there, and the literal converted with it.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
CREATE TABLE md (id INT, m mood DEFAULT 'q'::mood);
