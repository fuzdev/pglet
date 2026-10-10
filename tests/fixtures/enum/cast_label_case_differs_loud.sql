-- A label is compared as written and never folded: `SAD` is not the member `sad`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: SAD is not a member of mood
-- @expect-message
SELECT 'SAD'::mood::text AS v;
