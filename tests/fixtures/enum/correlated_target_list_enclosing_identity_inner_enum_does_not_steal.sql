-- The inner level carries a same-named column of **another enum**; the reference binds
-- the enclosing `text` column, so the literal beside the sub-plan is read as `text` and
-- the comparison answers. Read through the inner column it would be `22P02` for a label
-- that enum lacks, and label positions of the wrong enum (`EDGES.md` row 159).
-- @setup
CREATE TYPE eid_mood2 AS ENUM ('zzz', 'aaa');
CREATE TABLE eid_tx (m TEXT, k INT);
CREATE TABLE eid_tm2 (m eid_mood2, k INT);
INSERT INTO eid_tx VALUES ('zzz', 1), ('qqq', 2);
INSERT INTO eid_tm2 VALUES ('aaa', 1);
-- @test
SELECT k,
       (SELECT o.m FROM eid_tm2 b) = 'qqq' AS eq,
       'qqq' IN (SELECT o.m FROM eid_tm2 b) AS in_sub,
       coalesce((SELECT o.m FROM eid_tm2 b), 'qqq') AS folded,
       (SELECT o.m FROM eid_tm2 b) > 'aaa' AS gt
  FROM eid_tx o ORDER BY k;
