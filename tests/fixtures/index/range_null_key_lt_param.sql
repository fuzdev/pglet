-- `c < $1` bound through the extended protocol over a `NULLS NOT DISTINCT` index: the
-- NULL-keyed row is not in the range.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @param $1 int4 5
-- @test
SELECT v FROM rk WHERE c < $1;
