-- The range inside a scalar subquery, over a `NULLS NOT DISTINCT` index.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT (SELECT count(*) FROM rk WHERE c < 5) AS n;
