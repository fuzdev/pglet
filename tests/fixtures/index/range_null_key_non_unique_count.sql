-- `count(*)` over `c <= 5` and a non-unique `NULLS NOT DISTINCT` index holding two
-- NULL-keyed rows: the one row whose key is in the range.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (NULL, 'null2'), (1, 'one'), (10, 'ten');
-- @test
SELECT count(*) AS n FROM rk WHERE c <= 5;
