-- `addTargetToGroupList` coerces an `unknown` target-list entry to `text` when a
-- `GROUP BY` key **names that entry**, exactly as `addTargetToSortList` does for a sort
-- key — so a positional `GROUP BY 1` takes the unknown away and 18.6 refuses the
-- assignment.
--
-- Its accept twin is `assignment_cast_select_group_by_source_column_does_not_resolve.sql`,
-- which differs only in what the key names: a **source** column is its own group
-- expression and resolves nothing. The pair is what says the rule is
-- `findTargetlistEntrySQL92`'s claimant set and not "an aggregate node resolves
-- everything" — measured on 18.6, both ways.
-- @setup
CREATE TABLE acgo (id INT, b BOOL);
CREATE TABLE acgosrc (n INT);

-- @expect-error: column "b" is of type boolean but expression is of type text (42804)
-- @test
INSERT INTO acgo (b) SELECT 't' FROM acgosrc GROUP BY 1;
