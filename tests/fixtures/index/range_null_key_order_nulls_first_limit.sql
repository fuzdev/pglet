-- A range with no lower bound under `ORDER BY c NULLS FIRST LIMIT 1`: the first row is
-- the least key in the range, there being no NULL in it to place first.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5 ORDER BY c NULLS FIRST LIMIT 1;
