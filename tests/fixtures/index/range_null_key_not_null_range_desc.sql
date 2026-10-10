-- A range with no lower bound read in descending index order ends at the empty string.
-- @profile-lock
-- @setup
CREATE TABLE rk (c TEXT NOT NULL, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES ('b', 'b'), ('', 'empty'), ('a', 'a');
-- @test
SELECT v FROM rk WHERE c < 'b' ORDER BY c DESC;
