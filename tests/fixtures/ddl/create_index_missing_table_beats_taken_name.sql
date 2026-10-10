-- `DefineIndex` opens the target relation first and `index_create` tests the index name
-- last, so a missing table outranks a taken index name. pglet tested the name first.
-- @setup
CREATE TABLE cimt_t (a INT);
CREATE INDEX cimt_ix ON cimt_t (a);
-- @test
CREATE INDEX cimt_ix ON cimt_nothing (a);
-- @expect-error: the missing target outranks the taken index name (42P01)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
