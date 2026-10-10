-- A `WITH` query's `name(cols)` list renames the CTE relation's columns: the
-- list's names replace the body's positionally, on every reader (the wildcard
-- expansion, the binding gate, and the executed rows' own descriptor).
-- @setup
CREATE TABLE clr (a INT, b TEXT);
INSERT INTO clr VALUES (1, 'x'), (2, 'y');
-- @test
WITH c(p, q) AS (SELECT a, b FROM clr) SELECT p, q FROM c ORDER BY p;
