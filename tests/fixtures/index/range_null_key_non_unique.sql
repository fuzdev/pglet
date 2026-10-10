-- A `NULLS NOT DISTINCT` index need not be unique, and then holds an entry for each
-- NULL-keyed row. A range with no lower bound answers none of them.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (NULL, 'null2'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5 ORDER BY v;
