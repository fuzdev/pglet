-- A `uuid` key holding the all-zero value, its least, against a bound parameter.
-- @profile-lock
-- @setup
CREATE TABLE rk (c UUID, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('00000000-0000-0000-0000-000000000000', 'zero'), ('ffffffff-ffff-ffff-ffff-ffffffffffff', 'last');
-- @param $1 uuid 11111111-1111-1111-1111-111111111111
-- @test
SELECT v FROM rk WHERE c < $1;
