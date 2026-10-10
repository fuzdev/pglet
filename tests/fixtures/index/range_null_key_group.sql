-- Grouping the rows of a range by whether their key is NULL: one group, since the range
-- holds no NULL key.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT c IS NULL AS key_is_null, count(*) AS n FROM rk WHERE c < 5 GROUP BY 1 ORDER BY 1;
