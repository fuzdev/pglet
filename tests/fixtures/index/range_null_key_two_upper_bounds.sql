-- Two upper bounds on one column, one of them left as a residual filter: neither is
-- true of the NULL-keyed row a `NULLS NOT DISTINCT` index holds.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 7 AND c < 5;
