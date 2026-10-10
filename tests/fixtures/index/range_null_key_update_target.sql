-- An `UPDATE` whose own `WHERE` is the range rewrites the rows in it and leaves the
-- NULL-keyed row.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
UPDATE rk SET v = 'hit' WHERE c <= 10;
SELECT v FROM rk ORDER BY c;
