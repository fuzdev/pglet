-- A foreign key onto a unique `NULLS NOT DISTINCT` index refuses a child key no parent
-- holds.
-- @profile-lock
-- @expect-error: no parent holds the key
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
CREATE TABLE child (c INT REFERENCES rk (c), w TEXT);
-- @test
INSERT INTO child VALUES (2, 'orphan');
