-- A set operation inside a sub-plan whose side names an enclosing level's enum column
-- sorts its rows by member position: `sad` is the least of `{happy, sad}` for the first
-- row. The executor describes that side from the same record the planner reads; reading
-- the side's own input would sort by label bytes and answer `happy` (`EDGES.md` row 159).
-- @setup
CREATE TYPE eid_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE eid_te (m eid_mood, k INT);
CREATE TABLE eid_tz (z TEXT);
INSERT INTO eid_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO eid_tz VALUES ('a');
-- @test
SELECT k,
       (SELECT o.m FROM eid_tz UNION SELECT 'sad' ORDER BY 1 LIMIT 1)::text AS least_union,
       (SELECT o.m FROM eid_tz UNION ALL SELECT 'sad' ORDER BY 1 LIMIT 1)::text AS least_union_all
  FROM eid_te o ORDER BY k;
