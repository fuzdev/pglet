-- A foreign key onto a unique `NULLS NOT DISTINCT` index: a NULL child key references
-- nothing and is accepted, a present one finds its parent.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
CREATE TABLE child (c INT REFERENCES rk (c), w TEXT);
-- @test
INSERT INTO child VALUES (NULL, 'unreferenced'), (1, 'referenced');
SELECT w FROM child ORDER BY w;
