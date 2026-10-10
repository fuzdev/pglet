-- Plain `SELECT DISTINCT` groups on **every** target-list entry
-- (`transformDistinctClause` → `addTargetToGroupList` per entry), so it resolves every
-- `unknown` to `text` and 18.6 refuses the assignment.
--
-- Its accept twin is `assignment_cast_select_having_does_not_resolve_the_unknown.sql`.
-- The pair separates the two halves of the same aggregate/dedup family: `DISTINCT`
-- claims the entries, a `HAVING` claims none — so neither "this node resolves" nor
-- "this node does not" is a property of the node kind alone.
-- @setup
CREATE TABLE acdi (id INT, b BOOL);

-- @expect-error: column "b" is of type boolean but expression is of type text (42804)
-- @test
INSERT INTO acdi (b) SELECT DISTINCT 't';
