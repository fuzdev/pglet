-- The no-`FROM` spelling: `(SELECT eid_te.m)` names the target's enum column and orders
-- by member position, so the `DELETE` removes the `happy` row 18.6 removes; read as
-- `text` it would remove the `sad` one.
-- @setup
CREATE TYPE eid_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE eid_te (m eid_mood, k INT);
INSERT INTO eid_te VALUES ('happy', 1), ('sad', 2);
-- @test
DELETE FROM eid_te WHERE (SELECT eid_te.m) > 'ok' RETURNING k;
