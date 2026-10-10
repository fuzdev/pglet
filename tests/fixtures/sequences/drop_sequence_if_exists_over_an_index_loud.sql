-- The index half of the silent spelling: `IF EXISTS` over a wrong-kind relation is
-- still 42809, whichever kind holds the name.
-- @setup
CREATE TABLE dsiei_t (a INT);
CREATE INDEX dsiei_ix ON dsiei_t (a);
-- @test
DROP SEQUENCE IF EXISTS dsiei_ix;
-- @expect-error: an index is not a sequence, `IF EXISTS` notwithstanding (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
