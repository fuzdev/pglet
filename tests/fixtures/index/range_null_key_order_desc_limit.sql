-- A range with no lower bound under `ORDER BY c DESC LIMIT 1`. NULLs sort first
-- descending, so a NULL-keyed row left in the range would be the answer.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5 ORDER BY c DESC LIMIT 1;
