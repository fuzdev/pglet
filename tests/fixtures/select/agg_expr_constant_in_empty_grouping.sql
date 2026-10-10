-- A constant in a grouping query's select list is evaluated per group, so over an input
-- that forms no group it is never evaluated — and 18.6's planner never folds it here
-- either, the sub-select's output column being one nothing reads.
-- @setup
CREATE TABLE e (a INT, b INT);
-- @test
SELECT EXISTS (SELECT 1/0 FROM e GROUP BY a) AS no_group,
  (SELECT count(*) FROM (SELECT 1/0 AS z FROM e GROUP BY a) q) AS unread_output;
