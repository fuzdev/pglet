-- A qualified column reference whose qualifier names no in-scope table/alias is a
-- `42P01` (missing FROM-clause entry), NOT a silent resolve of the bare column.
-- `bogus.v` used to loosely match `tri_q`'s `v` and return its value; Postgres
-- rejects the query outright, and so must pglet (the silent-wrong-column bug).
-- @setup
CREATE TABLE tri_q (v INT);
INSERT INTO tri_q VALUES (1);

-- @test
-- @expect-error: missing FROM-clause entry for table "bogus" (42P01)
SELECT bogus.v FROM tri_q;
