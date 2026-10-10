-- The other half of the precedence pair: the column list also resolves ahead of every
-- cell's conversion, so an ill-typed cell behind a repeat does not get to answer first.
-- pglet reported the cell's `22P02` here.
-- @setup
CREATE TABLE dtccc (a INT, b INT);

-- @expect-error: column "a" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtccc (a, a) VALUES (1, 'zz');
