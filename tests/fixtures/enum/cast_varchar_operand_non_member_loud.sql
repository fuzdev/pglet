-- A `varchar` operand converts through its text, as 18.6's I/O coercion does: on the
-- value, so a row reaches it here.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT 'q'::varchar::mood::text AS v;
