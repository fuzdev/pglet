-- An `integer[]` key under a unique index, a B-tree. The empty array is the least key a
-- value has and is in the range; the NULL key is not.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT[], v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('{}', 'empty'), ('{1}', 'one'), ('{5}', 'five');
-- @param $1 int4[] {5}
-- @test
SELECT v FROM rk WHERE c < $1 ORDER BY v;
