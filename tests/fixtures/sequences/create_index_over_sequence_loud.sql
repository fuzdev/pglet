-- `DefineIndex` opens the target and switches on its relkind before anything else
-- (`cannot create index on relation "s"`).
-- @setup
CREATE SEQUENCE cis_seq;
-- @test
CREATE INDEX cis_ix ON cis_seq (a);
-- @expect-error: cannot create an index on a sequence (42809)
-- @no-profile-lock: DDL `@test` — index selection never runs, so both profiles share one path
