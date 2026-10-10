-- A DELETE keyed by `u = $1`, `$1` NULL, over a `NULLS NOT DISTINCT` unique index deletes
-- nothing — the index holds the NULL-keyed row under a NULL key, and no row's `u = NULL`
-- is true.
-- @profile-lock
-- @param $1 int4 NULL
-- @setup
CREATE TABLE nn (id INT, u INT);
CREATE UNIQUE INDEX nn_u ON nn (u) NULLS NOT DISTINCT;
INSERT INTO nn VALUES (1, NULL), (2, 5);
-- @test
DELETE FROM nn WHERE u = $1 RETURNING id;
