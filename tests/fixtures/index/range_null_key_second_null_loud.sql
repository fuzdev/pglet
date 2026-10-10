-- A second NULL key under a unique `NULLS NOT DISTINCT` index is a duplicate: the index
-- holds the first under a NULL key, whatever a range read leaves out.
-- @profile-lock
-- @expect-error: a second NULL key is a duplicate
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
INSERT INTO rk VALUES (NULL, 'again');
