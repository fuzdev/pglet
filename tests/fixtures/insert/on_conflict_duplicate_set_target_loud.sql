-- The `ON CONFLICT DO UPDATE SET` list is a set, under the same `42601` an `UPDATE`'s
-- `SET` list takes: `RewriteQuery` hands `onConflictSet` to the very
-- `rewriteTargetListIU` an `UPDATE` target list goes through. pglet accepted the list and
-- wrote the **last** value.
--
-- The member that fixes where the gate must sit is
-- `on_conflict_duplicate_set_target_without_a_conflict_loud.sql`, not this one.
-- @setup
CREATE TABLE ocd (a INT PRIMARY KEY, b INT);
INSERT INTO ocd VALUES (1, 1);

-- @expect-error: multiple assignments to same column "b" (42601)
-- @profile-lock
-- @test
INSERT INTO ocd VALUES (1, 9) ON CONFLICT (a) DO UPDATE SET b = 2, b = 3;
