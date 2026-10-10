-- The gate has to reach the INSERT value list too. Coercing each value to its target
-- column's type says nothing about whether the value expression is itself well-typed,
-- so this was accepted and written.
-- @setup
CREATE TABLE ivc (i4 INT4, tx TEXT);
-- @test
-- @expect-error: COALESCE branches have no common type (42804)
INSERT INTO ivc VALUES (COALESCE(1, 'x'::TEXT), 'y');
