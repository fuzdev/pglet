-- A `timestamptz` key against a bound parameter.
-- @profile-lock
-- @setup
CREATE TABLE rk (c TIMESTAMPTZ, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('2024-01-01 00:00:00+00', 'new year'), ('2025-01-01 00:00:00+00', 'later');
-- @param $1 timestamptz 2024-06-01T00:00:00Z
-- @test
SELECT v FROM rk WHERE c < $1;
