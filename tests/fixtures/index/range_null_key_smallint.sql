-- A `smallint` key against an `integer` literal, over a non-unique `NULLS NOT DISTINCT`
-- index.
-- @profile-lock
-- @setup
CREATE TABLE rk (c SMALLINT, v TEXT);
CREATE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES (NULL, 'null'), (NULL, 'null2'), (-32768, 'least'), (7, 'seven');
-- @test
SELECT v FROM rk WHERE c <= 0 ORDER BY v;
