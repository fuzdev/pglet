-- Controls for the enclosing-column reading: where the reference binds **locally** the
-- local column is its type. `o.m` over an inner `te o2` binds outward (the qualifier is
-- the outer alias) to a same-typed column; over an inner `te o` the inner alias shadows
-- the outer one and binds locally; a comparison inside the sub-plan and a reference in
-- its `WHERE` read the outer column at its own level.
-- @setup
CREATE TYPE ctl2_mood AS ENUM ('sad', 'ok', 'happy');
CREATE TABLE ctl2_te (m ctl2_mood, k INT);
CREATE TABLE ctl2_tt (m TEXT, k TEXT);
INSERT INTO ctl2_te VALUES ('happy', 1), ('sad', 2);
INSERT INTO ctl2_tt VALUES ('x', 'y');
-- @test
SELECT k,
       (SELECT o.m FROM ctl2_te o2 ORDER BY o2.k LIMIT 1) > 'ok' AS outward_same_type,
       (SELECT o.m FROM ctl2_te o ORDER BY o.k LIMIT 1) > 'ok' AS shadowed,
       (SELECT o.k = '1' FROM ctl2_tt b) AS inside_the_sub_plan,
       EXISTS (SELECT 1 FROM ctl2_tt b WHERE o.m > 'ok') AS in_its_where
  FROM ctl2_te o ORDER BY k;
