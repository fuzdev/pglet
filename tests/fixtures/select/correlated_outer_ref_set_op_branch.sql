-- A correlated outer reference inside **each arm of a set operation** within the
-- subquery. A set-op arm is its own query level whose parent is the query *containing*
-- the set operation, not the set-op node — the node introduces no relation name at
-- all — so the arms chain past it. Getting that wrong leaves both arms unresolved and
-- the literal uncoerced (`42883`, where Postgres returns the row).
-- @setup
CREATE TABLE cs_o (okey INT, ouid UUID);
CREATE TABLE cs_i (ikey INT, itag TEXT);
INSERT INTO cs_o VALUES (1, '11111111-1111-1111-1111-111111111111'), (2, '22222222-2222-2222-2222-222222222222');
INSERT INTO cs_i VALUES (1, 'a');
-- @test
SELECT okey FROM cs_o WHERE EXISTS (SELECT 1 FROM cs_i WHERE cs_o.ouid = '11111111-1111-1111-1111-111111111111' UNION ALL SELECT 2 FROM cs_i WHERE cs_o.ouid = '11111111-1111-1111-1111-111111111111') ORDER BY okey;
