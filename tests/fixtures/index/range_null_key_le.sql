-- `c <= 1` over a `NULLS NOT DISTINCT` index: the row whose key is 1, and not the
-- NULL-keyed row the index holds below it.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c <= 1;
