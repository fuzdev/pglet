-- `INSERT … SELECT` whose source is a range with no lower bound: it stores the rows in
-- the range, and the NULL-keyed row of the `NULLS NOT DISTINCT` index is not one.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
CREATE TABLE sink (v TEXT);
-- @test
INSERT INTO sink SELECT v FROM rk WHERE c < 5;
SELECT v FROM sink ORDER BY v;
