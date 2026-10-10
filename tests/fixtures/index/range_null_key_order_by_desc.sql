-- `ORDER BY c DESC` over a nullable key with no `WHERE`: every row, the NULL-keyed one
-- first.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk ORDER BY c DESC;
