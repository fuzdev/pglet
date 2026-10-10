-- A range with no lower bound over a `NULLS NOT DISTINCT` index answers the rows the
-- comparison is true of: the index holds the NULL-keyed row, and `c < 5` is not true of it.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5;
