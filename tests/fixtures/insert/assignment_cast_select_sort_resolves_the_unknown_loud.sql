-- The pair that fixes *where* the unknown carve-out may look. `addTargetToSortList`
-- coerces an `unknown` sort key to `text`, so adding an `ORDER BY` to the accepted
-- statement in `assignment_cast_select_unknown_literal_resolves.sql` makes 18.6 refuse
-- it — the two differ in nothing else.
--
-- Read as a constraint on the implementation: a helper that sees through **every**
-- row-order wrapper, a `Sort` among them, finds the bare `'t'` under the sort and
-- wrongly calls this assignment unknown. What separates the wrappers is not their
-- position but whether they claim a target-list entry — its pair-mates
-- `assignment_cast_select_limit_does_not_resolve_the_unknown.sql` (a wrapper that
-- claims none) and `assignment_cast_select_sort_marks_only_the_keyed_entry.sql` (a sort
-- claiming one entry of two) hold the other two corners.
-- @setup
CREATE TABLE acso (id INT, b BOOL);

-- @expect-error: column "b" is of type boolean but expression is of type text (42804)
-- @test
INSERT INTO acso (b) SELECT 't' ORDER BY 1;
