-- The same ordered read descending under a `LIMIT`.
-- @profile-lock
-- @setup
CREATE TABLE rk (c TEXT NOT NULL, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES ('b', 'b'), ('', 'empty'), ('a', 'a');
-- @test
SELECT v FROM rk ORDER BY c DESC LIMIT 2;
