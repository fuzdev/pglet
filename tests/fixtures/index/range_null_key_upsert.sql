-- `ON CONFLICT` arbitrates on the NULL key of a unique `NULLS NOT DISTINCT` index: the
-- second NULL updates the row that holds it.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
INSERT INTO rk VALUES (NULL, 'again') ON CONFLICT (c) DO UPDATE SET v = 'upserted' RETURNING v;
