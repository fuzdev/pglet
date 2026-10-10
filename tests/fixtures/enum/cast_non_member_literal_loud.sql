-- A cast of a string literal to an enum runs `enum_in` on it in parse analysis (EDGES.md
-- row 241): a label the enum does not hold is `22P02`. It answered the label as though a
-- member.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
SELECT 'q'::mood::text AS v;
