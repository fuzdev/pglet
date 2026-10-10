-- `ORDER BY` a nullable key with no `WHERE`: every row, the NULL-keyed one last. The
-- `NULLS NOT DISTINCT` index holds that row, and an ordered read of the index would not
-- return it, so the sort stays.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk ORDER BY c;
