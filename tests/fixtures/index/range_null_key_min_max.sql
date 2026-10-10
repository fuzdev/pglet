-- Aggregates over the whole table: `min` and `max` skip the NULL key, `count(*)` counts
-- its row.
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT min(c) AS least, max(c) AS greatest, count(*) AS every_row, count(c) AS keyed FROM rk;
