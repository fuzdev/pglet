-- An upper bound below every key: the range holds nothing, and the NULL key the
-- `NULLS NOT DISTINCT` index keeps below its values is not in it either.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT EXISTS (SELECT 1 FROM rk WHERE c < 0) AS found;
