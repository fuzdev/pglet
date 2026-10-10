-- Position: the heap (and its type test) is created before the foreign keys are
-- added, so the type namespace's `42710` outranks a missing parent's `42P01`.
-- @setup
CREATE TYPE tnt11 AS ENUM ('x');
-- @expect-error: the type collision outranks the missing FK parent
-- @expect-message
-- @test
CREATE TABLE tnt11 (a INT REFERENCES nosuch_tnt11 (a));
