-- An `UPDATE` whose predicate compares a scalar subquery naming the target's enum column
-- — over a one-column `int` inner table — updates the row 18.6 updates (`EDGES.md`
-- row 159).
-- @setup
CREATE TYPE eid_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE eid_te (m eid_mood, k INT);
CREATE TABLE eid_ti (z INT);
INSERT INTO eid_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO eid_ti VALUES (7);
-- @test
UPDATE eid_te SET k = k + 10 WHERE (SELECT eid_te.m FROM eid_ti) > 'ok' RETURNING k;
