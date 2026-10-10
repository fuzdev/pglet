-- `CREATE SEQUENCE IF NOT EXISTS` skips on the relation namespace only, so over an enum
-- it is the type namespace's `42710`, not the skip.
-- @setup
CREATE TYPE tnt5 AS ENUM ('x');
-- @expect-error: IF NOT EXISTS does not skip a sequence over a type
-- @expect-message
-- @test
CREATE SEQUENCE IF NOT EXISTS tnt5;
