-- A no-FROM select's `ORDER BY` key that is the same undeclared `$N` as an entry
-- resolves that entry to `text` before the set operation sees it
-- (`findTargetlistEntrySQL99`'s match).
-- @setup
CREATE TABLE ups (n INT);
INSERT INTO ups VALUES (1);
-- @param $1 int4 5
-- @expect-error: UNION types integer and text cannot be matched
-- @test
SELECT 1 UNION ALL (SELECT $1 ORDER BY $1);
