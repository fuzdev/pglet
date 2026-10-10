-- `truncate_check_rel` reports `"x" is not a table` for an index, so `TRUNCATE` over
-- one is `42809` rather than the `42P01` the shared table lookup used to report.
-- @setup
CREATE TABLE ttoi_t (a INT);
CREATE INDEX ttoi_ix ON ttoi_t (a);
-- @test
TRUNCATE ttoi_ix;
-- @expect-error: an index is not a table (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
