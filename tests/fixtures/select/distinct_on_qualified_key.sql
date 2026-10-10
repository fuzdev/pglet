-- @setup
CREATE TABLE staff (dept TEXT, name TEXT, salary INT);
INSERT INTO staff VALUES
  ('eng', 'alice', 100),
  ('eng', 'bob', 150),
  ('sales', 'dan', 90);

-- @test
-- The DISTINCT ON expr and the ORDER BY key denote the same column via different
-- spellings — qualified `staff.dept` vs bare `dept`. Postgres matches them by
-- column identity (not spelling), so this is not a mismatch: highest-paid per dept.
SELECT DISTINCT ON (staff.dept) dept, name, salary FROM staff ORDER BY dept, salary DESC;
