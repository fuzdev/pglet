-- A branch no row takes is still converted — the constant is folded where it is read, not
-- where it would be evaluated.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT (CASE WHEN false THEN 'q'::mood END)::text AS v;
