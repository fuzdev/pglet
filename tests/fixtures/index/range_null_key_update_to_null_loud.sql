-- An `UPDATE` that moves a row onto the NULL key a unique `NULLS NOT DISTINCT` index
-- already holds is a duplicate.
-- @profile-lock
-- @expect-error: the NULL key is taken
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
UPDATE rk SET c = NULL WHERE c = 1;
