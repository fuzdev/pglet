-- The rename reaches the **correlated** outer-scope chain, a third relation walk
-- (`push_level_relations`): the inner subquery resolves `c.p` against the enclosing
-- level's CTE relation.
-- @setup
CREATE TABLE clc (a INT);
CREATE TABLE clcs (k INT, m TEXT);
INSERT INTO clc VALUES (1), (2);
INSERT INTO clcs VALUES (1, 'one'), (2, 'two');
-- @test
WITH c(p) AS (SELECT a FROM clc)
SELECT p, (SELECT m FROM clcs WHERE clcs.k = c.p) AS corr FROM c ORDER BY p;
