-- `checkInsertTargets` is parse analysis and the `GENERATED ALWAYS` identity guard is a
-- rewriter check (`rewriteTargetListIU`), so the column list's `42701` outranks the
-- `428C9` that a non-`DEFAULT` value for an identity column earns. pglet reported the
-- `428C9`, its identity guard running inside the row source's own lowering.
--
-- The boundary in the other direction is `on_conflict_duplicate_set_target_loud.sql`'s
-- neighbour: the `INSERT`'s own target list is rewritten before the `ON CONFLICT` one,
-- so there the identity guard wins.
-- @setup
CREATE TABLE dtcg (id INT GENERATED ALWAYS AS IDENTITY, b INT);

-- @expect-error: column "id" specified more than once (42701)
-- @no-profile-lock: the column list is resolved while the row source is lowered, before any index selection runs
-- @test
INSERT INTO dtcg (id, id) VALUES (1, 2);
