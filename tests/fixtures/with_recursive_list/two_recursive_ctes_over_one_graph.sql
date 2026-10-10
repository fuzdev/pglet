-- Forward and backward reachability over one cyclic graph, two recursive CTEs
-- side by side: a working table shared between them would mix the two walks.
-- @setup
CREATE TABLE e (a INT, b INT);
INSERT INTO e VALUES (1, 2), (2, 3), (3, 1), (9, 10), (10, 11);
-- @test
WITH RECURSIVE fwd(n) AS (SELECT 1 UNION SELECT e.b FROM fwd JOIN e ON e.a = fwd.n),
  bwd(n) AS (SELECT 10 UNION SELECT e.a FROM bwd JOIN e ON e.b = bwd.n)
SELECT 'f' AS dir, n FROM fwd UNION ALL SELECT 'b', n FROM bwd ORDER BY 1, 2;
