-- Into a **text** column no enum gate guards: the label landed in `tx`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE tx (v TEXT);

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
INSERT INTO tx SELECT 'q'::mood;
