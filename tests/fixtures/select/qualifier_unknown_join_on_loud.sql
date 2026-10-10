-- A bogus table qualifier in a JOIN `ON` predicate is a `42P01` (missing
-- FROM-clause entry). Distinct from `comma_join_on_scope_loud` (an existing but
-- out-of-scope sibling): here `bogus` names no relation at all. The join scope's
-- resolver (`JoinColumnMap`) rejects the unknown qualifier rather than
-- loose-matching `.x`.
-- @setup
CREATE TABLE a (x INT);
CREATE TABLE b (x INT);
INSERT INTO a VALUES (1), (2);
INSERT INTO b VALUES (1), (2);

-- @test
-- @expect-error: missing FROM-clause entry for table "bogus" (42P01)
SELECT * FROM a JOIN b ON bogus.x = b.x;
