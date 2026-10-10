-- A constant standing beside a grouping query's aggregates is evaluated where the scalar
-- walk reaches it, as in a query that groups nothing: one in a branch 18.6's planner
-- folded away is never evaluated on either engine. The `substring` form here is one
-- pglet refuses, so evaluating a constant ahead of the rows would raise where 18.6
-- answers.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT a, CASE WHEN length('abc') = 3 THEN 1 ELSE 1/0 END AS c,
  length('abc') = 3 OR 1/0 > 0 AS o, coalesce(abs(1), 1/0) AS co,
  CASE WHEN a > 100 THEN substring('abc' from 'b.') ELSE 'x' END AS refused_form,
  count(*) AS n
FROM u GROUP BY a ORDER BY a;
