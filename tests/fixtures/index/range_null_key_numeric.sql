-- A `numeric` key: the negative and zero rows are below 1, the NULL-keyed row is not.
-- @profile-lock
-- @setup
CREATE TABLE rk (c NUMERIC, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (-1.5, 'negative'), (0, 'zero'), (2.5, 'positive');
-- @test
SELECT v FROM rk WHERE c < 1 ORDER BY c;
