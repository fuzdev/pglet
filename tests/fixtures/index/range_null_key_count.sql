-- `count(*)` over a range with no lower bound counts the rows the comparison is true
-- of — the NULL-keyed row of a `NULLS NOT DISTINCT` index is not one.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT count(*) AS every_row, count(c) AS keyed FROM rk WHERE c < 5;
