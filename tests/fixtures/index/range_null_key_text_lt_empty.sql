-- `c < ''` over a `text` key: nothing sorts below the empty string, and a NULL key is not
-- a value below it.
-- @profile-lock
-- @setup
CREATE TABLE rk (c TEXT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('', 'empty'), ('a', 'a');
-- @test
SELECT v FROM rk WHERE c < '';
