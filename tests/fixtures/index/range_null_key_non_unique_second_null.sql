-- A non-unique `NULLS NOT DISTINCT` index takes any number of NULL keys.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (NULL, 'null2'), (1, 'one'), (10, 'ten');
-- @test
INSERT INTO rk VALUES (NULL, 'null3');
SELECT count(*) AS n FROM rk WHERE c IS NULL;
