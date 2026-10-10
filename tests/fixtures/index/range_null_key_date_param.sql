-- A `date` key holding `-infinity`, its least value, against a bound parameter.
-- @profile-lock
-- @setup
CREATE TABLE rk (c DATE, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('-infinity', 'negative infinity'), ('2024-01-01', 'new year'), ('2025-01-01', 'later');
-- @param $1 date 2024-06-01
-- @test
SELECT v FROM rk WHERE c < $1 ORDER BY c;
