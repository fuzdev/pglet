-- The same boundary through the `WHERE`, which reaches the re-walk by a different
-- clause. Both spellings were `42601` here while the gate sat above that pass.
-- @setup
CREATE TABLE dscw (a INT, b INT);
CREATE TABLE dscwe (b INT);
INSERT INTO dscw VALUES (0, 0);

-- @expect-error: column "zzz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dscw SET a = 1, a = 2 WHERE (SELECT b FROM dscwe ORDER BY zzz) = 1;
