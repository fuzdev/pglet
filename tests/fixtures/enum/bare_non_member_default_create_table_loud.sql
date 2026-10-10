-- The bare literal: an `unknown` into an enum column is converted by `enum_in` as the
-- default is cooked (EDGES.md row 6's invalid-label member). The DDL was accepted and every
-- insert taking the default refused.
-- @setup
CREATE TYPE mood AS ENUM ('sad', 'ok', 'happy');

-- @test
-- @expect-error: q is not a member of mood
-- @expect-message
CREATE TABLE md (id INT, m mood DEFAULT 'q');
