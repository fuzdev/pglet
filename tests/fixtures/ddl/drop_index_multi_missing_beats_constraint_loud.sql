-- The constraint-owned refusal is a PHASE-2 check, so a later name's phase-1 failure
-- outranks it: `42704` (a missing index), not the `2BP01` the first name earns alone.
-- Mirrored by `drop_index_multi_constraint_owned_reverse_loud.sql`, which is that
-- `2BP01` over the same first name — the pair is what pins the phase split, since
-- `@expect-error` compares only the SQLSTATE.
-- @setup
CREATE TABLE dixmb (a INT PRIMARY KEY, b INT);

-- @expect-error: index "dixmb_nosuch" does not exist
-- @no-profile-lock: the @test is DDL — it takes no query plan, so index acceleration cannot change its outcome
-- @test
DROP INDEX dixmb_pkey, dixmb_nosuch;
