-- `transformWhereClause` is parse analysis and the duplicate check is a rewriter check,
-- so the `WHERE`'s own `42703` outranks it. The same boundary the identity guard below
-- this gate already had.
-- @setup
CREATE TABLE dstw (a INT, b INT);
INSERT INTO dstw VALUES (0, 0);

-- @expect-error: column "zz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dstw SET a = 1, a = 2 WHERE zz = 1;
