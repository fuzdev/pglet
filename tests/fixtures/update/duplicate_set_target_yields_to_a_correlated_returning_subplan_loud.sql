-- The gate's last boundary: the correlated re-walk, which is where a sub-plan's own
-- binding gate closes. That gate stands in for parse analysis, and a rewriter check
-- outranks nothing parse analysis refuses — so the re-walk's `42703` wins, at one row
-- and at none. Placed above the re-walk the gate answered `42601` here instead.
-- @setup
CREATE TABLE dscr (a INT, b INT);
CREATE TABLE dscre (b INT);
INSERT INTO dscr VALUES (0, 0);

-- @expect-error: column "zzz" does not exist (42703)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dscr SET a = 1, a = 2 RETURNING (SELECT b FROM dscre ORDER BY zzz);
