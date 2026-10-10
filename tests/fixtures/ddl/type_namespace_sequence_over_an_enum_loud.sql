-- A sequence has no row type, but `heap_create_with_catalog` asks the type namespace of
-- every relation it creates — so a sequence over an enum's name is `42710` too, the
-- direction a "relations with a row type" reading of the rule misses.
-- @setup
CREATE TYPE tnt4 AS ENUM ('x');
-- @expect-error: a sequence over an enum's name
-- @expect-message
-- @test
CREATE SEQUENCE tnt4;
