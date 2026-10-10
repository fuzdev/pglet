-- Position: a column default is cooked once the heap exists, so the type namespace's
-- `42710` outranks the default's own `22P02`.
-- @setup
CREATE TYPE tnt13 AS ENUM ('x');
-- @expect-error: the type collision outranks the default cook
-- @expect-message
-- @test
CREATE TABLE tnt13 (a INT DEFAULT 'x');
