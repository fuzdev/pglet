-- `c <= $1` bound over a non-unique `NULLS NOT DISTINCT` index holding two NULL-keyed
-- rows.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (NULL, 'null2'), (1, 'one'), (10, 'ten');
-- @param $1 int4 10
-- @test
SELECT v FROM rk WHERE c <= $1 ORDER BY v;
