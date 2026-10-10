-- An `ARRAY[…]` element is a cast like any other: the non-member second element is
-- refused beside a member first one.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT ARRAY['ok'::mood, 'q'::mood]::text AS v;
