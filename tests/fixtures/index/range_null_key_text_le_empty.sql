-- `c <= ''` over a `text` key: the empty string's row, and not the NULL-keyed row beside
-- it in the `NULLS NOT DISTINCT` index.
-- @profile-lock
-- @setup
CREATE TABLE rk (c TEXT, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), ('', 'empty'), ('a', 'a');
-- @test
SELECT v FROM rk WHERE c <= '';
