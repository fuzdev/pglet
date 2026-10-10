-- A containment probe whose array is of another element type than the indexed column's
-- has no operator, with a GIN index over the column as without one: the probe's keys
-- are not the index's, and an empty posting list is not the answer.
-- @profile-lock
-- @setup
CREATE TABLE ckx_g (id INT, n NUMERIC[]);
CREATE INDEX ckx_g_ix ON ckx_g USING gin (n);
INSERT INTO ckx_g (n, id) VALUES ('{1.0,2}', 1);
-- @test
-- @expect-error: numeric[] @> integer[] has no operator (42883)
SELECT id FROM ckx_g WHERE n @> ARRAY[1];
