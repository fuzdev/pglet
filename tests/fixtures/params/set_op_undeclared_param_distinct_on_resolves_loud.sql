-- A `DISTINCT ON` key that is the same undeclared `$N` as an entry resolves that entry
-- to `text` before the set operation sees it (`findTargetlistEntrySQL99`'s match).
-- @setup
CREATE TABLE ups (n INT);
INSERT INTO ups VALUES (1);
-- @param $1 int4 5
-- @expect-error: UNION types integer and text cannot be matched
-- @test
SELECT 1 UNION ALL SELECT DISTINCT ON ($1) $1 FROM ups;
