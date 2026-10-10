-- A `text` key. The empty string is the least key a value has and is in `c < 'b'`; the
-- NULL key the `NULLS NOT DISTINCT` index holds below it is not.
-- @profile-lock
-- @setup
CREATE TABLE rk (c TEXT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('', 'empty'), ('a', 'a'), ('b', 'b');
-- @test
SELECT v FROM rk WHERE c < 'b' ORDER BY v;
