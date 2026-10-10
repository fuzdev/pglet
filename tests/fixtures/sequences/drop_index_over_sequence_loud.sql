-- `DROP INDEX` reads the relkind from the other side — `"s" is not an index` — where
-- pglet reported 42704 (`does not exist`) for a name it would have refused to reuse.
-- @setup
CREATE SEQUENCE dis_seq;
-- @test
DROP INDEX dis_seq;
-- @expect-error: a sequence is not an index (42809)
