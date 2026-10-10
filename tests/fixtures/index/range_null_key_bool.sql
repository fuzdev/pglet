-- A `boolean` key over a non-unique `NULLS NOT DISTINCT` index: `c < true` is the `false`
-- rows.
-- @profile-lock
-- @setup
CREATE TABLE rk (c BOOLEAN, v TEXT);
CREATE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (false, 'no'), (true, 'yes'), (NULL, 'null2');
-- @test
SELECT v FROM rk WHERE c < true;
