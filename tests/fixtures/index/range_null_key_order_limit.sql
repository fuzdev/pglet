-- A range with no lower bound under `ORDER BY c LIMIT 2` over a nullable key: one row is
-- in the range, so one row answers — the NULL-keyed row is not there to sort last.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5 ORDER BY c LIMIT 2;
