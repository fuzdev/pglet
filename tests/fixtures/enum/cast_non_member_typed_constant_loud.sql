-- The typed-constant spelling `mood 'q'` — `gram.y`'s `ConstTypename Sconst`, the same
-- conversion of the same literal.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT (mood 'q')::text AS v;
