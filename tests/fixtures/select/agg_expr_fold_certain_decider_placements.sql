-- A certain decider drops an aggregate wherever the grouping evaluates an expression:
-- the select list, `HAVING` and an `ORDER BY` key alike, and through nested connectives —
-- a connective of two certain operands is itself one.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT false OR (true OR sum(1/b) > 0) AS nested,
  (true AND false) OR (1 = 1 OR sum(1/b) > 0) AS chained,
  (NULL OR 2 > 1) OR sum(1/b) > 0 AS null_or_true,
  (NULL AND 2 < 1) AND sum(1/b) > 0 AS null_and_false,
  count(*) AS n
FROM u HAVING 1 = 1 OR sum(1/b) > 0
ORDER BY CASE WHEN 1 = 1 THEN count(*) ELSE sum(1/b) END;
