-- A one-directional FK, parent written first. Dropping the parent alone is `2BP01`;
-- naming the child in the same statement removes the blocker.
-- @setup
CREATE TABLE dofp (id INT PRIMARY KEY);
CREATE TABLE dofpc (x INT REFERENCES dofp(id));
CREATE TABLE dofpkeep (z INT);

-- @test
DROP TABLE dofp, dofpc;
SELECT count(*)::int AS remaining FROM information_schema.tables
WHERE table_schema = 'public'
  AND (table_name = 'dofp'
       OR table_name = 'dofpc'
       OR table_name = 'dofpkeep');
