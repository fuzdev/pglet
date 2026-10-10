-- The `INSERT`'s own target list is rewritten **before** the `ON CONFLICT` one
-- (`RewriteQuery` calls `rewriteTargetListIU` on the main target list, then on
-- `onConflictSet`), so a non-`DEFAULT` value for a `GENERATED ALWAYS` identity column
-- answers ahead of the action's duplicate. Both engines report the `428C9`.
--
-- The opposite boundary — the action's *own* identity guard, which 18.6 reaches after
-- the merge and pglet reaches before it — is the residual `EDGES.md` records.
-- @setup
CREATE TABLE ocdi (id INT GENERATED ALWAYS AS IDENTITY PRIMARY KEY, b INT);
INSERT INTO ocdi (b) VALUES (7);

-- @expect-error: cannot insert a non-DEFAULT value into column "id" (428C9)
-- @profile-lock
-- @test
INSERT INTO ocdi (id, b) VALUES (1, 2) ON CONFLICT (id) DO UPDATE SET b = 1, b = 2;
