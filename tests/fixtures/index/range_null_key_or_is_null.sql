-- A range `OR`ed with a null test answers the range's rows and the NULL-keyed row.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5 OR c IS NULL ORDER BY v;
