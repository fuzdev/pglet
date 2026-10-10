-- An `interval` key holding a negative span, against a typed literal.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INTERVAL, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('-1 day', 'yesterday'), ('1 hour', 'hour');
-- @test
SELECT v FROM rk WHERE c < INTERVAL '1 minute';
