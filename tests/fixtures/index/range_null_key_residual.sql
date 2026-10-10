-- A range beside a conjunct on another column. The NULL-keyed row would pass the
-- residual `v <> 'one'`, so only the range keeps it out.
-- @profile-lock
-- @setup
CREATE TABLE rk (c INT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (1, 'one'), (10, 'ten');
-- @test
SELECT v FROM rk WHERE c < 5 AND v <> 'one';
