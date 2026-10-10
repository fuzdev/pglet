-- The comparison written value-first, `5 > c`, over a `NULLS NOT DISTINCT` index: an
-- upper bound on `c`, true of no NULL.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE 5 > c;
