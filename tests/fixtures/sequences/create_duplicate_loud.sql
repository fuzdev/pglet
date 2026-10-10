-- A second `CREATE SEQUENCE` of the same name is 42P07. pglet used to `or_insert`,
-- which silently discarded the new START WITH and kept the old counter.
-- @setup
CREATE SEQUENCE dup_seq START WITH 5;
-- @test
CREATE SEQUENCE dup_seq START WITH 50;
-- @expect-error: relation already exists (42P07)
