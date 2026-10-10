-- `c < $1` with `$1` NULL over a `NULLS NOT DISTINCT` index: the comparison is NULL for
-- every row, the NULL-keyed one included, so no row answers.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @param $1 int4 NULL
-- @test
SELECT v FROM rk WHERE c < $1;
