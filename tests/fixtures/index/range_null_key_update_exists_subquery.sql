-- An `UPDATE` under `EXISTS` over a range that holds no row writes nothing — the
-- NULL key below the range's values does not make it non-empty.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
CREATE TABLE sink (v TEXT);
INSERT INTO sink VALUES ('null'), ('one'), ('ten');
-- @test
UPDATE sink SET v = 'hit' WHERE EXISTS (SELECT 1 FROM rk WHERE c < 0);
SELECT v FROM sink ORDER BY v;
