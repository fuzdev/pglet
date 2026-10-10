-- The committed walk of a non-unique `NULLS NOT DISTINCT` index built over rows already
-- there, two of them NULL-keyed.
-- @profile-lock
-- @no-txn
-- @setup
DROP TABLE IF EXISTS rk_ac_nu;
CREATE TABLE rk_ac_nu (c INT, v TEXT);
INSERT INTO rk_ac_nu VALUES (NULL, 'null'), (NULL, 'null2'), (1, 'one'), (10, 'ten');
CREATE INDEX rk_ac_nu_c ON rk_ac_nu (c) NULLS NOT DISTINCT;
-- @test
SELECT v FROM rk_ac_nu WHERE c <= 10 ORDER BY v;
