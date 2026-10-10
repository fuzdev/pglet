-- A `bigint` key holding its least value: in the range, where the NULL key is not.
-- @profile-lock
-- @setup
CREATE TABLE rk (c BIGINT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (-9223372036854775808, 'least'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5 ORDER BY c;
