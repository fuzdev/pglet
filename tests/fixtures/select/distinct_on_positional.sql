-- @setup
CREATE TABLE emp (dept TEXT, name TEXT, salary INT);
INSERT INTO emp VALUES
  ('eng', 'alice', 100),
  ('eng', 'bob', 150),
  ('sales', 'dan', 90),
  ('sales', 'eve', 200);

-- @test
-- `DISTINCT ON (1)` and `ORDER BY 1` both resolve to the first output column
-- (`dept`), like ORDER BY's ordinal form — highest-paid per dept, addressed
-- positionally.
SELECT DISTINCT ON (1) dept, name, salary FROM emp ORDER BY 1, salary DESC;
