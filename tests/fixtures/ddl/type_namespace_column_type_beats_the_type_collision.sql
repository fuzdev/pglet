-- The other side of the position: a column type is resolved in parse analysis, ahead of
-- the heap, so a missing type's `42704` outranks the type namespace's `42710`.
-- @setup
CREATE TYPE tnt14 AS ENUM ('x');
-- @expect-error: the column type outranks the type collision
-- @test
CREATE TABLE tnt14 (a nosuchtype_tnt14);
