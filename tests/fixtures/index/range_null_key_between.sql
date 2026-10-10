-- `BETWEEN` over a `NULLS NOT DISTINCT` index: the key inside, and neither the key
-- below the lower bound nor the NULL key.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c BETWEEN 5 AND 20;
