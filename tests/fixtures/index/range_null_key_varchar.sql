-- A `varchar(n)` key holding the empty string.
-- @profile-lock
-- @setup
CREATE TABLE rk (c VARCHAR(5), v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('', 'empty'), ('a', 'a'), ('b', 'b');
-- @test
SELECT v FROM rk WHERE c < 'b' ORDER BY c;
