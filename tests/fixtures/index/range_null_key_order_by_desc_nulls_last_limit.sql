-- `ORDER BY c DESC NULLS LAST LIMIT 2` over a nullable key with no `WHERE`.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk ORDER BY c DESC NULLS LAST LIMIT 2;
