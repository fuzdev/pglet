-- The other position of the duplicate-beats-identity boundary: the repeat is on an
-- ordinary column and the identity column is assigned behind it. `rewriteTargetListIU`
-- merges every target entry before it walks the relation's identity columns, so the
-- repeat still answers first — where pglet's identity guard, running last in the
-- lowering, answered `428C9`. Its sibling repeats the identity column itself
-- (`duplicate_set_target_beats_the_identity_guard_loud.sql`); neither position alone
-- says the whole merge pass precedes the whole identity pass.
-- @setup
CREATE TABLE dsia (id INT GENERATED ALWAYS AS IDENTITY, b INT);
INSERT INTO dsia (b) VALUES (7);

-- @expect-error: multiple assignments to same column "b" (42601)
-- @no-profile-lock: the refusal is raised while the statement is lowered, before any index selection runs
-- @test
UPDATE dsia SET b = 1, b = 2, id = 3;
