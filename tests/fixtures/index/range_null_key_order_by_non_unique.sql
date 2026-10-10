-- `ORDER BY` a nullable key under a non-unique `NULLS NOT DISTINCT` index: both
-- NULL-keyed rows, last.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (NULL, 'null2'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk ORDER BY c, v;
