-- @setup
CREATE TABLE emp (dept TEXT, name TEXT, salary INT);
INSERT INTO emp VALUES
  ('eng', 'alice', 100),
  ('eng', 'bob', 150),
  ('eng', 'carol', 120),
  ('sales', 'dan', 90),
  ('sales', 'eve', 200);

-- @test
-- `DISTINCT ON (dept)` keeps the first row of each dept group in the query's
-- ORDER BY order; ORDER BY dept, salary DESC makes that the highest-paid employee
-- per department. The foreign `salary DESC` tiebreak follows the ON key in ORDER BY.
SELECT DISTINCT ON (dept) dept, name, salary FROM emp ORDER BY dept, salary DESC;
