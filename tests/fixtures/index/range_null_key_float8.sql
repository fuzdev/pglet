-- A `double precision` key. `-Infinity` is the least value and is in `c <= 0`; the NULL
-- key is not, and `NaN` sorts above every number.
-- @profile-lock
-- @setup
CREATE TABLE rk (c DOUBLE PRECISION, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('-Infinity', 'negative infinity'), (0, 'zero'), ('NaN', 'nan');
-- @test
SELECT v FROM rk WHERE c <= 0 ORDER BY c;
