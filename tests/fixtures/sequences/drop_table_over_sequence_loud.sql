-- `DROP TABLE <a sequence>` is the wrong object type, not a missing one — pglet said
-- 42P01, which denied the existence of a name `CREATE TABLE` would have refused.
-- @setup
CREATE SEQUENCE dts_seq;
-- @test
DROP TABLE dts_seq;
-- @expect-error: a sequence is not a table (42809)
