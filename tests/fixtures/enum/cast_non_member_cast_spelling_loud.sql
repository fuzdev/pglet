-- The `CAST(… AS …)` spelling of `cast_non_member_literal_loud` — one builder, one rule.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT CAST('q' AS mood)::text AS v;
