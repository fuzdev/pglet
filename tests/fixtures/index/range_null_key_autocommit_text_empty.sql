-- The committed walk over a `text` key: `c <= ''` is the empty string's row, the least
-- key a value has, and not the NULL-keyed row.
-- @profile-lock
-- @no-txn
-- @setup
DROP TABLE IF EXISTS rk_ac_te;
CREATE TABLE rk_ac_te (c TEXT, v TEXT);
CREATE UNIQUE INDEX rk_ac_te_c ON rk_ac_te (c) NULLS NOT DISTINCT;
INSERT INTO rk_ac_te VALUES (NULL, 'null'), ('', 'empty'), ('a', 'a');
-- @test
SELECT v FROM rk_ac_te WHERE c <= '';
