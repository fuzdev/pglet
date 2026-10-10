-- The rename reaches `*` expansion, which reads the relation's columns through the
-- plan-time namespace rather than through the projection.
-- @setup
CREATE TABLE clw (a INT, b TEXT);
INSERT INTO clw VALUES (1, 'x'), (2, 'y');
-- @test
WITH c(p, q) AS (SELECT a, b FROM clw) SELECT * FROM c ORDER BY p;
