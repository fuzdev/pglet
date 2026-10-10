-- A sequence is a relation in `pg_class`, so it shares one name space with tables
-- and indexes: creating a table over a sequence's name is 42P07, exactly as it is
-- over another table's. pglet used to keep sequences in their own map, so both
-- statements succeeded and the name meant two things.
-- @setup
CREATE SEQUENCE ns_seq_first;
-- @test
CREATE TABLE ns_seq_first (a INT);
-- @expect-error: relation already exists (42P07)
