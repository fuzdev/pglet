-- A simple `CASE` whose operand and arm value are integer or `numeric` literals folds as
-- 18.6 folds it (`CASE 0 WHEN 0 THEN 1 ELSE 1/0 END` is its planner's own example): an
-- arm whose value is unequal is dropped, and the arms after an equal one are.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT CASE 1 WHEN 1 THEN 0 ELSE sum(1/b) END AS m,
  CASE 1 WHEN 2 THEN sum(1/b) ELSE 7 END AS n,
  CASE 1 WHEN 1.0 THEN 8 ELSE sum(1/b) END AS int_numeric,
  CASE 1.5 WHEN 2 THEN sum(1/b) WHEN 1.5 THEN 9 WHEN 3 THEN sum(1/b) ELSE sum(1/b) END AS later_arms,
  CASE 1 WHEN count(*) THEN 3 WHEN 1 THEN 4 ELSE sum(1/b) END AS behind_an_aggregate_arm
FROM u;
