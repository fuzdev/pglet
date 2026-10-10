-- Every right-hand side is transformed before any target name is assigned, and all of it
-- is parse analysis — so an unbindable right-hand side outranks the rewriter's duplicate
-- check.
-- @setup
CREATE TABLE dstr (a INT, b INT);
INSERT INTO dstr VALUES (0, 0);

-- @expect-error: column "zz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dstr SET a = zz, a = 2;
