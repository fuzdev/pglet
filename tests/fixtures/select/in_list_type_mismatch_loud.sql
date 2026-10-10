-- `IN (…)` is a chain of `=`, so a typed element with no operator against the
-- left operand is `42883`. (A bare *string* literal in the list is the accepted
-- unknown-literal case — see `types/compare_in_list_bad_literal_loud.sql`.)
-- @setup
CREATE TABLE t (name TEXT);
INSERT INTO t VALUES ('5');
-- @test
-- @expect-error: no text = integer operator
SELECT * FROM t WHERE name IN (5, 6);
