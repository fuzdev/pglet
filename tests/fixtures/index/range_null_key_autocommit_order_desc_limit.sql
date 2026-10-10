-- The committed walk under `ORDER BY c DESC LIMIT 1`, where a NULL-keyed row left in the
-- range would sort first.
-- @profile-lock
-- @no-txn
-- @setup
DROP TABLE IF EXISTS rk_ac_od;
CREATE TABLE rk_ac_od (c INT, v TEXT);
CREATE UNIQUE INDEX rk_ac_od_c ON rk_ac_od (c) NULLS NOT DISTINCT;
INSERT INTO rk_ac_od VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk_ac_od WHERE c < 5 ORDER BY c DESC LIMIT 1;
