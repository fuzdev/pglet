-- `DropErrorMsgWrongType` words the refusal against the *expected* kind, so an index
-- named to `DROP TABLE` is `42809` — not the `42P01` a tables-map-only lookup reports
-- for a name the very next statement would refuse to reuse.
-- @setup
CREATE TABLE dtoi_t (a INT);
CREATE INDEX dtoi_ix ON dtoi_t (a);
-- @test
DROP TABLE dtoi_ix;
-- @expect-error: an index is not a table (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
