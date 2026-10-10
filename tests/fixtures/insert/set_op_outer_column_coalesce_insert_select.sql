-- It writes: the `INSERT … SELECT` source's `COALESCE` over a set operation naming the
-- source's `numeric` column is `numeric`, and stores `1.25`.
-- @setup
CREATE TABLE socs (n NUMERIC);
CREATE TABLE soct (n NUMERIC);
INSERT INTO socs VALUES (1.25);
-- @test
INSERT INTO soct (n) SELECT coalesce((SELECT n EXCEPT SELECT 1), 2) FROM socs RETURNING n;
