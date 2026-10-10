-- The zero-row twin of `join_using_match_before_bad_loud.sql`: a `USING` column whose
-- two sides have no comparison operator is `42883` in Postgres with both tables
-- empty, because the resolution is on declared types. A per-row gate never fires here.
-- @setup
CREATE TABLE a (k TEXT, av TEXT);
CREATE TABLE b (k INT, bv TEXT);
-- @test
-- @expect-error: no text = integer operator for the USING column, with no rows
SELECT av, bv FROM a JOIN b USING (k);
