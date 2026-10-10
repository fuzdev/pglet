-- A range that holds no row, under `ORDER BY c LIMIT 1`: no row, where a NULL-keyed row
-- left in the range would be the one the limit keeps.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 0 ORDER BY c LIMIT 1;
