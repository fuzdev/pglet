-- @setup
CREATE TABLE crew (dept TEXT, name TEXT, salary INT);
INSERT INTO crew VALUES
  ('eng', 'alice', 100),
  ('eng', 'bob', 150),
  ('sales', 'dan', 90);

-- @test
-- The DISTINCT ON expr is the base column `dept`; the ORDER BY key is its output
-- alias `d` (`dept AS d`). They resolve to the same output column, so Postgres
-- matches them by identity rather than treating `d` as a foreign sort key.
SELECT DISTINCT ON (dept) dept AS d, name, salary FROM crew ORDER BY d, salary DESC;
