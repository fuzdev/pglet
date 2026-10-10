-- Both this gate and the `GENERATED ALWAYS` identity guard stand in for rewriter checks,
-- and `rewriteTargetListIU` runs them in one order: it merges **every** target entry in
-- its first loop and only walks the relation's identity columns in its second. So the
-- duplicate wins, whether the repeated column is the identity one or another —
-- `SET b = 1, b = 2, id = 3` is this same `42601`. pglet answered `428C9` to both, its
-- identity guard having been the last thing the lowering did.
-- @setup
CREATE TABLE dsig (id INT GENERATED ALWAYS AS IDENTITY, b INT);
INSERT INTO dsig (b) VALUES (7);

-- @expect-error: multiple assignments to same column "id" (42601)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dsig SET id = 1, id = 2;
