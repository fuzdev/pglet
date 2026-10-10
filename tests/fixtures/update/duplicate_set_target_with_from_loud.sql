-- An `UPDATE … FROM` reaches the same refusal: the `FROM` relation changes what the
-- right-hand sides can name and nothing about the target list, which is rewritten the
-- same way. pglet accepted it and wrote through the join.
-- @setup
CREATE TABLE dstf (a INT, b INT);
CREATE TABLE dstfs (a INT, v INT);
INSERT INTO dstf VALUES (1, 0);
INSERT INTO dstfs VALUES (1, 9);

-- @expect-error: multiple assignments to same column "a" (42601)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dstf SET a = 5, a = 6 FROM dstfs WHERE dstfs.a = dstf.a;
