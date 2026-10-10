-- A plain index and a `NULLS NOT DISTINCT` one over the same column, the latter's name
-- sorting first: whichever serves the range, the NULL-keyed row is not in it.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE INDEX rk_b ON rk (c);
CREATE INDEX rk_a ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5;
