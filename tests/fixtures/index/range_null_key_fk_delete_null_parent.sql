-- Deleting the NULL-keyed parent of a foreign key onto a unique `NULLS NOT DISTINCT`
-- index is unrestricted: no child key equals a NULL.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
CREATE TABLE child (c INT REFERENCES rk (c), w TEXT);
INSERT INTO child VALUES (NULL, 'unreferenced'), (1, 'referenced');
-- @test
DELETE FROM rk WHERE c IS NULL;
SELECT v FROM rk ORDER BY v;
