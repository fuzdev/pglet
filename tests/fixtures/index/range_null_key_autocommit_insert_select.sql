-- `INSERT … SELECT` from a range over the committed index, autocommitted: the stored
-- rows are read back after the statement commits.
-- @profile-lock
-- @no-txn
-- @setup
DROP TABLE IF EXISTS rk_ac_is_sink;
DROP TABLE IF EXISTS rk_ac_is;
CREATE TABLE rk_ac_is (c INT, v TEXT);
CREATE UNIQUE INDEX rk_ac_is_c ON rk_ac_is (c) NULLS NOT DISTINCT;
INSERT INTO rk_ac_is VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
CREATE TABLE rk_ac_is_sink (v TEXT);
-- @test
INSERT INTO rk_ac_is_sink SELECT v FROM rk_ac_is WHERE c < 5;
SELECT v FROM rk_ac_is_sink ORDER BY v;
