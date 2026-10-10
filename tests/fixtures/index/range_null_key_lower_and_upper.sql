-- Both bounds, exclusive, over a `NULLS NOT DISTINCT` index: the key between them.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (5, 'five'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c > 1 AND c < 10;
