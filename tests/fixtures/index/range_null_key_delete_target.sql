-- A `DELETE` whose own `WHERE` is the range removes the rows in it and leaves the
-- NULL-keyed row.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
DELETE FROM rk WHERE c < 5;
SELECT v FROM rk ORDER BY v;
