-- A `DO $$ … $$` body runs its statements through the executor directly, below both
-- hosts, so the `25001` gate the hosts ask had a third site that never asked it:
-- wrapping the statement in a `DO` built the index, **including inside an open
-- transaction block**, where the bare statement is correctly refused.
--
-- 18.6 refuses it from a function unconditionally (`PreventInTransactionBlock`), and
-- so does pglet now. The harness's own wrapper is a block, which is the case whose
-- message the two engines share (`cannot run inside a transaction block`; with no
-- block open 18.6 says `cannot be executed from a function` under the same
-- SQLSTATE).
-- @setup
CREATE TABLE ti (c INT);
-- @test
-- @no-profile-lock: the `25001` refusal precedes execution and builds no index, so
-- index selection never runs on either profile — the completeness check is reading
-- the `CREATE INDEX` in this comment block, not one in the setup.
-- @expect-error: CREATE INDEX CONCURRENTLY cannot run inside a transaction block
DO $$ BEGIN CREATE INDEX CONCURRENTLY ti_c_idx ON ti (c); END $$;
