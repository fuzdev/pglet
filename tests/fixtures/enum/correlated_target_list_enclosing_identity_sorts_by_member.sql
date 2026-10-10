-- A scalar subquery naming an enclosing level's enum column sorts and deduplicates by
-- member position — directly, through a derived table's `max`, and through a CTE's
-- comparison: `sad` before `happy`, where the label bytes put `happy` first (`EDGES.md`
-- row 159).
-- @setup
CREATE TYPE eid_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE eid_te (m eid_mood, k INT);
CREATE TABLE eid_tz (z TEXT);
INSERT INTO eid_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO eid_tz VALUES ('a');
-- @test
WITH c AS (SELECT k, (SELECT o.m FROM eid_tz b) AS m FROM eid_te o)
SELECT d.v::text AS v,
       (SELECT max(m) FROM (SELECT (SELECT i.m FROM eid_tz b) AS m FROM eid_te i) x)::text AS mx,
       (SELECT k FROM c WHERE m > 'ok') AS above_ok
  FROM (SELECT DISTINCT (SELECT o.m FROM eid_tz b) AS v FROM eid_te o) d ORDER BY d.v;
