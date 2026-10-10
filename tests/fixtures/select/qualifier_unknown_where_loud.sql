-- A bogus table qualifier in a WHERE predicate is a `42P01` (missing FROM-clause
-- entry), not a silent bare-name resolve. `bogus.v = 1` must fail loud like
-- Postgres rather than folding to `v = 1`.
-- @setup
CREATE TABLE tri_q (v INT);
INSERT INTO tri_q VALUES (1);

-- @test
-- @expect-error: missing FROM-clause entry for table "bogus" (42P01)
SELECT v FROM tri_q WHERE bogus.v = 1;
