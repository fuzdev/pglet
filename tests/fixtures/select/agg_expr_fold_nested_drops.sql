-- A drop nested in a drop ends without ending the outer one: what follows the inner
-- dropped branch, still inside the outer, is dropped too. Each `sum(1/b)` here stands
-- after an inner drop and inside an outer one, so 18.6's planner has folded it away and
-- the statement answers; collecting it would divide by the `b = 0` row.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT CASE WHEN false THEN coalesce(CASE WHEN false THEN 1 END, sum(1/b)) END AS case_in_case,
  true OR ((false AND max(a) > 0) OR sum(1/b) > 0) AS and_in_or,
  coalesce(1, CASE WHEN true THEN 2 ELSE max(a) END, sum(1/b)) AS case_in_coalesce,
  count(*) AS n
FROM u;
