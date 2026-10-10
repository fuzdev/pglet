-- Keyed on the column, not on the spelling: `a` and `"a"` are one name after the
-- parser's fold, and 18.6's own key is the resolved attribute.
-- @setup
CREATE TABLE dstq (a INT, b INT);
INSERT INTO dstq VALUES (0, 0);

-- @expect-error: multiple assignments to same column "a" (42601)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dstq SET a = 3, "a" = 4;
