-- The no-FROM `INSERT … SELECT … WHERE` compared directly: a `WHERE` that is false writes
-- no row, and its command tag says so.
-- @setup
CREATE TABLE nfw (a int NOT NULL, b text);
-- @test
INSERT INTO nfw (a, b) SELECT 1, 'x' WHERE NOT EXISTS (SELECT 1) RETURNING a, b;
