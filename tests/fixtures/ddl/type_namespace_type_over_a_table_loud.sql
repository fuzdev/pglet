-- The other direction: an enum over a table's name meets the table's row type —
-- `DefineEnum`'s `42710`, which carries no hint.
-- @setup
CREATE TABLE tnt3 (a INT);
-- @expect-error: an enum over a table's name
-- @expect-message
-- @test
CREATE TYPE tnt3 AS ENUM ('x');
