-- **This is the member that fixes the gate's position.** The refusal is a rewriter check
-- and the rewriter runs whether or not any row conflicts, so an `INSERT` whose key is
-- free is refused just as the conflicting spelling is. A gate hung on the conflict path
-- leaves this statement **inserting** — which is what pglet did, adding the row and
-- reporting `INSERT 0 1`.
--
-- Its conflicting twin is `on_conflict_duplicate_set_target_loud.sql`; that one is red
-- under a conflict-path gate too, but only by SQLSTATE, where this one is red by having
-- written.
-- @setup
CREATE TABLE ocdf (a INT PRIMARY KEY, b INT);
INSERT INTO ocdf VALUES (1, 1);

-- @expect-error: multiple assignments to same column "b" (42601)
-- @profile-lock
-- @test
INSERT INTO ocdf VALUES (5, 1) ON CONFLICT (a) DO UPDATE SET b = 2, b = 3;
