-- The other polarity of the shared relation namespace: a sequence over an existing
-- table's name is 42P07 too.
-- @setup
CREATE TABLE ns_tbl_first (a INT);
-- @test
CREATE SEQUENCE ns_tbl_first;
-- @expect-error: relation already exists (42P07)
