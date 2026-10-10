-- A `text` operand is not a literal — `'q'::text` is already typed — so 18.6 converts it on
-- the value, at evaluation. With a row to evaluate, the non-member is `22P02`.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT 'q'::text::mood::text AS v;
