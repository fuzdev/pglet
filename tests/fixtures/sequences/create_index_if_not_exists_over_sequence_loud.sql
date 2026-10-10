-- The `IF NOT EXISTS` skip lives in `index_create`, at the *end* of `DefineIndex`, so
-- the target's relkind (checked right after `table_open`) beats it: a taken index name
-- does not turn this into a no-op. pglet tested the name first and answered success —
-- the same silent-skip shape as `DROP TABLE IF EXISTS` over a sequence.
-- @setup
CREATE SEQUENCE cins_seq;
CREATE TABLE cins_t (a INT);
CREATE INDEX cins_ix ON cins_t (a);
-- @test
CREATE INDEX IF NOT EXISTS cins_ix ON cins_seq (a);
-- @expect-error: the target's kind outranks the IF NOT EXISTS skip (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
