-- `ComputeIndexAttrs` also runs ahead of `index_create`, so an unresolvable key column
-- outranks both the name conflict and the `IF NOT EXISTS` skip — this statement errors
-- rather than reporting a skipped no-op.
-- @setup
CREATE TABLE cibc_t (a INT);
CREATE INDEX cibc_ix ON cibc_t (a);
-- @test
CREATE INDEX IF NOT EXISTS cibc_ix ON cibc_t (nosuchcol);
-- @expect-error: the key column outranks the IF NOT EXISTS skip (42703)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
