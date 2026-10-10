-- And a repeated column name (`MergeAttributes`, before the heap) outranks it too.
-- @setup
CREATE TYPE tnt15 AS ENUM ('x');
-- @expect-error: the duplicate column outranks the type collision
-- @expect-message
-- @test
CREATE TABLE tnt15 (a INT, a INT);
