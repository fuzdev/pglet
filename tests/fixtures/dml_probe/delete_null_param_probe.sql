-- A NULL key matches no row: `k = $1` bound NULL is NULL for every row, through the probe
-- as through the scan — the NULL-keyed row included.
-- @profile-lock
-- @param $1 int4 NULL
-- @setup
CREATE TABLE np (k INT, v TEXT);
CREATE INDEX np_k ON np (k);
INSERT INTO np VALUES (1, 'a'), (NULL, 'n');
-- @test
DELETE FROM np WHERE k = $1 RETURNING k, v;
