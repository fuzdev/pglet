-- A `real` key against a `numeric` literal the key type holds exactly.
-- @profile-lock
-- @setup
CREATE TABLE rk (c REAL, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (-1.5, 'negative'), (0, 'zero'), ('NaN', 'nan');
-- @test
SELECT v FROM rk WHERE c <= 0.5 ORDER BY c;
