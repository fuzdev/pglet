-- A `CASE` arm whose condition is the literal `false` or `NULL`, or a comparison of two
-- number literals that does not hold, is removed by 18.6's planner before its `Agg` node
-- collects the aggregates, so the division inside the dropped arm's `sum` never runs.
-- The table holds a `b = 0` row: an aggregate the arm failed to drop raises `22012`.
-- @setup
CREATE TABLE u (a INT, b INT NOT NULL, y TEXT);
INSERT INTO u VALUES (1, 10, 'x'), (2, 0, 'z');
-- @test
SELECT CASE WHEN false THEN sum(1/b) END AS f,
  CASE WHEN NULL THEN sum(1/b) ELSE 0 END AS n,
  CASE WHEN 1 > 2 THEN sum(1/b) END AS c,
  CASE WHEN 1 > 2 THEN sum(1/b) WHEN NULL THEN sum(1/b) WHEN false THEN sum(1/b) ELSE count(*) END AS every_arm
FROM u;
