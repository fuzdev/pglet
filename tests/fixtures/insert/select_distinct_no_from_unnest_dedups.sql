-- **The write polarity of the discarded `DISTINCT`: an extra row, silent on both engines.**
-- `INSERT INTO … SELECT DISTINCT unnest(ARRAY[1,1,2])` wrote **three** rows here against
-- 18.6's two, with no error anywhere — the same shape as the discarded `LIMIT`'s writes
-- and unlike the sort-key members, which are all loud on 18.6.
--
-- The assertion is the surviving row *set*, which is what makes this fixture independent
-- of the unspecified dedup output order its read sibling
-- (`select/no_from_distinct_dedups_an_unnest.sql`) has to sort around.
-- @setup
CREATE TABLE dni (a INT);
INSERT INTO dni SELECT DISTINCT unnest(ARRAY[1, 1, 2]);

-- @test
SELECT a FROM dni ORDER BY a;
