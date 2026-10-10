-- `ORDER BY` a `NOT NULL` key under a `NULLS NOT DISTINCT` index is read in index order,
-- from the least key a value has — the empty string.
-- @profile-lock
-- @setup
CREATE TABLE rk (c TEXT NOT NULL, v TEXT);
CREATE UNIQUE INDEX rk_c ON rk (c) NULLS NOT DISTINCT;
INSERT INTO rk VALUES ('b', 'b'), ('', 'empty'), ('a', 'a');
-- @test
SELECT v FROM rk ORDER BY c;
