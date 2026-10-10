-- A `DELETE` whose predicate compares a scalar subquery naming the target's enum column
-- removes the row 18.6 removes: `happy` > `ok` by member position. Read through the
-- sub-plan's own input it would remove the `sad` row (`EDGES.md` row 159).
-- @setup
CREATE TYPE eid_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE eid_te (m eid_mood, k INT);
CREATE TABLE eid_tz (z TEXT);
INSERT INTO eid_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO eid_tz VALUES ('a');
-- @test
DELETE FROM eid_te WHERE (SELECT eid_te.m FROM eid_tz) > 'ok' RETURNING k;
