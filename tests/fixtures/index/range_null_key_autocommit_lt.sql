-- The range read over the committed index, outside any block: `c < 5` answers no
-- NULL-keyed row of a `NULLS NOT DISTINCT` index.
-- @profile-lock
-- @no-txn
-- @setup
DROP TABLE IF EXISTS rk_ac_lt;
CREATE TABLE rk_ac_lt (c INT, v TEXT);
CREATE UNIQUE INDEX rk_ac_lt_c ON rk_ac_lt (c) NULLS NOT DISTINCT;
INSERT INTO rk_ac_lt VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk_ac_lt WHERE c < 5;
